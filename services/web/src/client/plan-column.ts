/// The plan column (ov-875): what the plan says, as values, the same answer
/// the Mac's AgentKit `PlanColumn` gives (ov-867, C10f), so the browser and
/// the Mac read one plan one way. The mental model is
/// `.claude/agent/reports/ov-867/intent.md`:
///
/// 1. the facts line: what needs you, what's stuck, the next landing, and how
///    much of the board is done;
/// 2. Initiatives (today's themes): the larger goals, each with progress and
///    its next step;
/// 3. the pipeline: status, then agent, then tickets. An agent says what it
///    does now and its tickets are listed under it; a ticket's initiative is
///    the ticket's, never the agent's, and each ticket is on one agent;
/// 4. the footer counts, which open the ticket lists.
///
/// Needs You sits between the facts and Initiatives; its rows are the
/// runner's own list, which the plan doesn't carry, so the page passes them in.
///
/// Nothing here touches the page, so a test plays a plan by hand. The DOM is
/// plan-view.ts.

import { isLiveTrain, needsAttention, type Counts, type FleetAgent, type Lane, type Phase, type Plan, type Theme, type Train } from './plan'

// MARK: Words

/// How long, coarsely: "45 min", "2 h", "3 d" (the Mac's `PlanWords.span`).
export function span(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / 60_000)
  if (minutes < 60) return `${minutes} min`
  if (minutes < 1440) return `${Math.floor(minutes / 60)} h`
  return `${Math.floor(minutes / 1440)} d`
}

/// A stale lane's warning, beside its mark: "No move in 2 h".
function staleWords(lane: Lane, now: number): string | null {
  if (!lane.stale) return null
  const minutes = Math.floor(Math.max(0, now - (lane.stateSince ?? now)) / 60_000)
  return minutes < 120 ? 'No move in an hour' : `No move in ${Math.floor(minutes / 60)} h`
}

/// A train's phase inside a sentence (the Mac's `TrainPhase.sentence`).
function phaseSentence(phase: Phase): string {
  switch (phase) {
    case 'waiting_for_lanes':
      return 'waiting for lanes'
    case 'waiting_for_previous_train':
      return 'waiting for the earlier landing'
    case 'waiting_for_main':
      return 'waiting for main’s checks to pass'
    case 'integrating':
    case 'gating_locally':
      return 'checking here'
    case 'waiting_for_ci':
      return 'checking on CI'
    case 'ci_failed':
      return 'CI failed'
    case 'ready_to_land':
      return 'ready to land'
    case 'landed':
      return 'landed'
    case 'stalled':
      return 'stalled'
    case 'dropped':
      return 'dropped'
    case 'unknown':
      return 'unknown'
  }
}

/// How far along a phase is, to find the train furthest along.
function progress(phase: Phase): number {
  switch (phase) {
    case 'landed':
    case 'dropped':
      return 8
    case 'ready_to_land':
      return 7
    case 'waiting_for_ci':
      return 6
    case 'ci_failed':
      return 5
    case 'gating_locally':
      return 4
    case 'integrating':
      return 3
    case 'waiting_for_lanes':
    case 'waiting_for_previous_train':
    case 'waiting_for_main':
      return 2
    case 'stalled':
      return 1
    case 'unknown':
      return 0
  }
}

/// What a lander says it does: its train's phase as a sentence, the
/// orchestrator's label first when it gave one and nothing is wrong.
function phaseWords(phase: Phase, label: string): { doing: string; trouble: boolean } {
  if (label && !needsAttention(phase)) return { doing: label, trouble: false }
  const sentence = phaseSentence(phase)
  return { doing: sentence.charAt(0).toUpperCase() + sentence.slice(1), trouble: needsAttention(phase) }
}

// MARK: Types

export interface Run {
  text: string
  strong: boolean
}

export type FactKind = 'needsYou' | 'stuck' | 'nextLanding' | 'done'

export interface Fact {
  kind: FactKind
  runs: Run[]
  /// Drawn in amber: it needs the owner (above 0) or is stuck.
  attention: boolean
}

export interface Initiative {
  id: string
  name: string
  /// The outcome the theme was written for; empty when it has none.
  goal: string
  done: number
  total: number
  /// What's next for it, or what waits on the owner (`asksOwner`).
  next: string | null
  asksOwner: boolean
  /// Its Next line as the runner has it, for editing.
  nextRaw: string
}

export const progressWords = (i: Initiative): string => `${i.done} of ${i.total}`
export const fraction = (i: Initiative): number => (i.total > 0 ? Math.min(1, i.done / i.total) : 0)

export type Stage = 'stuck' | 'building' | 'reviewing' | 'landing'
export const STAGES: Stage[] = ['stuck', 'building', 'reviewing', 'landing']
export const STAGE_TITLE: Record<Stage, string> = { stuck: 'Stuck', building: 'Building', reviewing: 'Reviewing', landing: 'Landing' }

export type Role = 'builder' | 'reviewer' | 'lander' | 'agent'
const ROLE_WORD: Record<Role, string> = { builder: 'Builder', reviewer: 'Reviewer', lander: 'Lander', agent: 'Agent' }

function roleOf(role: FleetAgent['role']): Role {
  switch (role) {
    case 'build':
    case 'fix':
      return 'builder'
    case 'review':
      return 'reviewer'
    case 'integrator':
      return 'lander'
    default:
      return 'agent'
  }
}

export interface Ticket {
  task: string
  key: string
  title: string
  /// The initiative the ticket is for, if any: the ticket's own.
  initiative: string | null
  stage: Stage
}

export interface Agent {
  id: string
  role: Role
  /// "Builder 2": its role, numbered when there are several.
  name: string
  doing: string
  /// `doing` says why it's stuck: drawn in amber.
  doingIsTrouble: boolean
  /// How long it has been at it, in ms; null when nothing says.
  elapsed: number | null
  /// Its pane, to open its chat; null when the runner names none.
  terminal: string | null
  tickets: Ticket[]
  /// It is working now: its glyph shows it.
  working: boolean
  /// Tickets in this stage nobody is on, said under the last card.
  more: number
}

export interface Group {
  stage: Stage
  agents: Agent[]
  /// Tickets in this stage with no agent on them.
  unattended: Ticket[]
  /// The tickets in this stage, each once.
  count: number
  /// "12 more waiting for review".
  unattendedWords: string | null
}

export type CountKind = 'waiting' | 'notStarted' | 'done'

export interface Count {
  kind: CountKind
  count: number
  /// "10 tickets waiting for a builder", "22 not started", "796 done".
  words: string
}

export interface Column {
  facts: Fact[]
  /// The initiatives in motion or waiting on the owner, in the board's order.
  initiatives: Initiative[]
  /// The rest that are open, said on one quiet line.
  quiet: Initiative[]
  pipeline: Group[]
  footer: Count[]
  now: number
  /// How many initiatives are open.
  initiativeCount: number
  agentCount: number
  /// "10 agents", "1 agent".
  agentWords: string
  /// "Nothing moving: Mac look and feel, 33 of 34".
  quietLine: string | null
}

export type Statuses = ReadonlyMap<string, string>

// MARK: Statuses

/// A board status from the plan's card word: "In Progress" is `in_progress`,
/// "Cancelled" and "Canceled" are `cancelled`.
export function statusOf(word: string): string {
  const w = word.trim().toLowerCase().replace(/\s+/g, '_')
  if (w === 'to_do') return 'todo'
  if (w === 'canceled') return 'cancelled'
  return w
}

/// The statuses the plan carries, by task.
export function statusesOf(plan: Plan): Map<string, string> {
  const out = new Map<string, string>()
  for (const c of plan.cards) if (c.status) out.set(c.task, statusOf(c.status))
  return out
}

const total = (c: Counts): number => c.backlog + c.todo + c.needsDecision + c.inProgress + c.inReview + c.done

// MARK: Building it

/// The themes as the overview lists them: active, then paused, then done,
/// each by its order on the board.
function shownThemes(plan: Plan): Theme[] {
  const rank: Record<string, number> = { active: 0, paused: 1, done: 2 }
  return plan.themes
    .filter(t => t.state !== 'dropped')
    .map((t, i) => ({ t, i }))
    .sort((a, b) => (rank[a.t.state] ?? 3) - (rank[b.t.state] ?? 3) || a.t.ordinal - b.t.ordinal || a.i - b.i)
    .map(x => x.t)
}

function initiatives(plan: Plan, moving: Set<string>): { shown: Initiative[]; quiet: Initiative[] } {
  const shown: Initiative[] = []
  const quiet: Initiative[] = []
  for (const theme of shownThemes(plan)) {
    if (theme.state === 'done') continue
    const sum = total(theme.counts)
    if (sum > 0 && theme.counts.done >= sum) continue
    const ask = theme.ask.trim()
    const next = theme.next.trim()
    const i: Initiative = {
      id: theme.id,
      name: theme.name,
      goal: theme.outcome.trim(),
      done: theme.counts.done,
      total: sum,
      next: ask ? `Waiting on you: ${ask}` : next ? `Next: ${next}` : null,
      asksOwner: ask !== '',
      nextRaw: theme.next,
    }
    const inMotion = theme.tasks.some(t => moving.has(t))
    // Paused themes are open but not in motion: they go to the quiet line.
    if ((inMotion && theme.state === 'active') || ask) shown.push(i)
    else quiet.push(i)
  }
  return { shown, quiet }
}

/// Tickets on a queued lane, waiting for a builder, each once and none
/// cancelled.
function waitingTickets(plan: Plan, statuses: Statuses): { task: string; key: string }[] {
  const seen = new Set<string>()
  const out: { task: string; key: string }[] = []
  for (const lane of plan.lanes) {
    if (lane.state !== 'queued') continue
    for (const c of lane.cardRefs) {
      if (statuses.get(c.task) === 'cancelled' || seen.has(c.task)) continue
      seen.add(c.task)
      out.push(c)
    }
  }
  return out
}

function footer(plan: Plan, statuses: Statuses): Count[] {
  const waiting = waitingTickets(plan, statuses)
  const counts: Count[] = []
  if (waiting.length > 0) {
    counts.push({ kind: 'waiting', count: waiting.length, words: `${waiting.length === 1 ? '1 ticket' : `${waiting.length} tickets`} waiting for a builder` })
  }
  const c = plan.boardCounts
  if (c) {
    // Not started, less the ones already counted as waiting.
    const unstarted = waiting.filter(t => ['backlog', 'todo'].includes(statuses.get(t.task) ?? 'todo')).length
    const notStarted = Math.max(0, c.backlog + c.todo - unstarted)
    if (notStarted > 0) counts.push({ kind: 'notStarted', count: notStarted, words: `${notStarted} not started` })
    if (c.done > 0) counts.push({ kind: 'done', count: c.done, words: `${c.done} done` })
  }
  return counts
}

/// The live train furthest along: how many tickets it carries, and since
/// when it has been in its phase.
function nextLanding(plan: Plan): { tickets: number; since: number | null } | null {
  const live = plan.trains.filter(t => isLiveTrain(t) && t.phase !== 'ci_failed' && t.phase !== 'stalled')
  let best: Train | null = null
  for (const t of live) if (!best || progress(t.phase) > progress(best.phase)) best = t
  if (!best) return null
  const ids = new Set(best.lanes.map(l => l.id))
  const seen = new Set<string>()
  for (const lane of plan.lanes) {
    if (ids.has(lane.id) && lane.state === 'landing') for (const c of lane.cardRefs) seen.add(c.task)
  }
  return { tickets: seen.size, since: best.phaseSince !== null && best.phaseSince > 0 ? best.phaseSince : null }
}

function facts(plan: Plan, needsYou: number, pipeline: Group[]): Fact[] {
  const out: Fact[] = []
  const run = (text: string, strong = false): Run => ({ text, strong })
  if (needsYou > 0) out.push({ kind: 'needsYou', runs: [run(`${needsYou}`, true), run(' need you')], attention: true })
  const stuck = pipeline.find(g => g.stage === 'stuck')
  if (stuck) {
    const times = stuck.agents.map(a => a.elapsed).filter((e): e is number => e !== null)
    const longest = times.length ? Math.max(...times) : null
    out.push({ kind: 'stuck', runs: [run(`${stuck.count} stuck${longest !== null ? `, ${span(longest)}` : ''}`)], attention: true })
  }
  const next = nextLanding(plan)
  if (next) {
    out.push({
      kind: 'nextLanding',
      runs: [
        run('Next landing: '),
        run(`${next.tickets}`, true),
        run(next.tickets === 1 ? ' ticket' : ' tickets'),
        ...(next.since !== null ? [run(`, ${span(plan.nowMs - next.since)}`)] : []),
      ],
      attention: false,
    })
  }
  const c = plan.boardCounts
  if (c && total(c) > 0) out.push({ kind: 'done', runs: [run(`${c.done}`, true), run(` of ${total(c)} done`)], attention: false })
  return out
}

interface Built {
  stage: Stage
  agent: Agent
  started: number
  /// The task of the card the agent's own record names.
  card: string | null
}

function pipelineOf(plan: Plan, statuses: Statuses): Group[] {
  const isTrainAgent = new Set(plan.laneIsTrain)
  const lanes = plan.lanes.filter(l => !isTrainAgent.has(l.id) && l.state !== 'landed' && l.state !== 'dropped' && l.state !== 'queued')
  const live = plan.trains.filter(isLiveTrain)
  const trainOf = new Map<string, Train>()
  for (const t of live) for (const l of t.lanes) trainOf.set(l.id, t)
  const cardOf = new Map(plan.cards.map(c => [c.task, c]))
  // The theme a task is in: the first open one.
  const themeOf = (task: string): Theme | undefined => plan.themes.find(t => t.state !== 'dropped' && t.tasks.includes(task))

  const ticket = (ref: { task: string; key: string }, lane: Lane | null, stage: Stage): Ticket => {
    const known = cardOf.get(ref.task)?.title
    let title = ref.key
    if (known) title = known
    else if (lane && lane.cardRefs.length === 1 && lane.title) title = lane.heading
    return { task: ref.task, key: ref.key, title, initiative: themeOf(ref.task)?.name ?? null, stage }
  }
  const stageOf = (lane: Lane): Stage => {
    const train = trainOf.get(lane.id)
    if (lane.state === 'landing' && train) return needsAttention(train.phase) ? 'stuck' : 'landing'
    if (lane.stale) return 'stuck'
    switch (lane.state) {
      case 'review':
        return 'reviewing'
      case 'landing':
        return 'landing'
      default:
        return 'building'
    }
  }
  const ticketsOf = (lane: Lane): Ticket[] => {
    const s = stageOf(lane)
    return lane.cardRefs.filter(c => statuses.get(c.task) !== 'cancelled').map(c => ticket(c, lane, s))
  }

  const cards: Built[] = []
  const attended = new Set<string>()
  const trainsDrawn = new Set<string>()
  for (const a of plan.agents) {
    if (a.role === 'orchestrator') continue
    const lane = a.lane ? lanes.find(l => l.id === a.lane) : undefined
    const train = a.train ? live.find(t => t.id === a.train) : undefined
    let role = roleOf(a.role)
    let mine: Ticket[] = []
    let stage: Stage
    let doing = a.label
    let trouble = false
    if (train) {
      // The agent landing a train: the lander, over the train's tickets.
      role = 'lander'
      trainsDrawn.add(train.id)
      const ids = new Set(train.lanes.map(l => l.id))
      mine = lanes.filter(l => ids.has(l.id) && l.state === 'landing').flatMap(ticketsOf)
      stage = needsAttention(train.phase) ? 'stuck' : 'landing'
      ;({ doing, trouble } = phaseWords(train.phase, a.label))
    } else if (lane) {
      mine = ticketsOf(lane)
      stage = stageOf(lane)
      const stale = stage === 'stuck' ? staleWords(lane, plan.nowMs) : null
      if (stale) {
        doing = stale
        trouble = true
      } else if (!doing) doing = lane.heading
    } else {
      stage = role === 'reviewer' ? 'reviewing' : role === 'lander' ? 'landing' : 'building'
      if (a.card) mine = [ticket(a.card, null, stage)]
      if (!doing) doing = a.working ? 'Working' : 'Idle'
    }
    mine = mine.map(t => ({ ...t, stage }))
    for (const t of mine) attended.add(t.task)
    cards.push({
      stage,
      started: a.startedAt,
      card: a.card?.task ?? null,
      agent: {
        id: a.id,
        role,
        name: ROLE_WORD[role],
        doing,
        doingIsTrouble: trouble,
        elapsed: a.startedAt > 0 ? Math.max(0, plan.nowMs - a.startedAt) : null,
        terminal: a.terminal,
        tickets: mine,
        working: a.working,
        more: 0,
      },
    })
  }
  // A live train whose integrating agent isn't listed: its own record says
  // who lands it, so it still has a lander.
  for (const train of live) {
    if (trainsDrawn.has(train.id) || !train.integrator?.working) continue
    const ids = new Set(train.lanes.map(l => l.id))
    const stage: Stage = needsAttention(train.phase) ? 'stuck' : 'landing'
    const mine = lanes.filter(l => ids.has(l.id) && l.state === 'landing').flatMap(ticketsOf).map(t => ({ ...t, stage }))
    if (mine.length === 0) continue
    for (const t of mine) attended.add(t.task)
    const { doing, trouble } = phaseWords(train.phase, '')
    const started = train.integrator.startedAt
    cards.push({
      stage,
      started,
      card: null,
      agent: {
        id: `train:${train.id}`,
        role: 'lander',
        name: ROLE_WORD.lander,
        doing,
        doingIsTrouble: trouble,
        elapsed: started > 0 ? Math.max(0, plan.nowMs - started) : null,
        terminal: null,
        tickets: mine,
        working: true,
        more: 0,
      },
    })
  }

  // Each ticket is on one agent: the lander of its train first, then the
  // agent whose own card it is, then the first to claim it.
  const owner = new Map<string, { rank: number; index: number }>()
  cards.forEach((card, index) => {
    for (const t of card.agent.tickets) {
      const rank = card.agent.role === 'lander' ? 0 : card.card === t.task ? 1 : 2
      const have = owner.get(t.task)
      if (have && (have.rank < rank || (have.rank === rank && have.index <= index))) continue
      owner.set(t.task, { rank, index })
    }
  })
  cards.forEach((card, index) => {
    card.agent.tickets = card.agent.tickets.filter(t => owner.get(t.task)?.index === index)
  })

  // Number each role in the order its agents started: "Builder 1".
  const byRole = new Map<Role, number[]>()
  cards.forEach((card, index) => byRole.set(card.agent.role, [...(byRole.get(card.agent.role) ?? []), index]))
  for (const [role, indices] of byRole) {
    if (indices.length < 2) continue
    const ordered = [...indices].sort((a, b) => cards[a].started - cards[b].started || a - b)
    ordered.forEach((index, n) => (cards[index].agent.name = `${ROLE_WORD[role]} ${n + 1}`))
  }

  // Tickets nobody is on, by the stage of their lane.
  const unattended = new Map<Stage, Ticket[]>()
  const seen = new Set(attended)
  for (const lane of lanes) {
    for (const t of ticketsOf(lane)) {
      if (seen.has(t.task)) continue
      seen.add(t.task)
      unattended.set(t.stage, [...(unattended.get(t.stage) ?? []), t])
    }
  }

  const groups: Group[] = []
  for (const stage of STAGES) {
    // Agents with tickets first, then in start order.
    const agents = cards
      .map((c, i) => ({ c, i }))
      .filter(x => x.c.stage === stage)
      .sort((a, b) => Number(a.c.agent.tickets.length === 0) - Number(b.c.agent.tickets.length === 0) || a.c.started - b.c.started || a.i - b.i)
      .map(x => x.c.agent)
    const loose = unattended.get(stage) ?? []
    // Said under the last card that has tickets, never under an empty one.
    for (let i = agents.length - 1; i >= 0; i--) {
      if (agents[i].tickets.length > 0) {
        agents[i].more = loose.length
        break
      }
    }
    if (agents.length === 0 && loose.length === 0) continue
    const tasks = new Set([...agents.flatMap(a => a.tickets.map(t => t.task)), ...loose.map(t => t.task)])
    groups.push({ stage, agents, unattended: loose, count: tasks.size, unattendedWords: unattendedWords(stage, loose.length, agents.length === 0) })
  }
  return groups
}

function unattendedWords(stage: Stage, n: number, noAgents: boolean): string | null {
  if (n === 0) return null
  const lead = noAgents ? (n === 1 ? '1 ticket' : `${n} tickets`) : `${n} more`
  switch (stage) {
    case 'stuck':
      return `${lead} with no agent on ${n === 1 ? 'it' : 'them'}`
    case 'building':
      return `${lead} with no builder running`
    case 'reviewing':
      return `${lead} waiting for review`
    case 'landing':
      return `${lead} waiting to land`
  }
}

/// The plan column for `plan`. `needsYou` is the workspace's Needs You count,
/// the initiatives' asks included. `statuses` are the board's card statuses
/// by task; the plan's own cards stand in for any the page doesn't pass.
export function columnOf(plan: Plan, needsYou = 0, statuses: Statuses = statusesOf(plan)): Column {
  const pipeline = pipelineOf(plan, statuses)
  const moving = new Set(pipeline.flatMap(g => [...g.agents.flatMap(a => a.tickets), ...g.unattended]).map(t => t.task))
  const { shown, quiet } = initiatives(plan, moving)
  const agentCount = pipeline.reduce((n, g) => n + g.agents.length, 0)
  return {
    facts: facts(plan, needsYou, pipeline),
    initiatives: shown,
    quiet,
    pipeline,
    footer: footer(plan, statuses),
    now: plan.nowMs,
    initiativeCount: shown.length + quiet.length,
    agentCount,
    agentWords: agentCount === 1 ? '1 agent' : `${agentCount} agents`,
    quietLine: quiet.length ? `Nothing moving: ${quiet.map(i => `${i.name}, ${progressWords(i)}`).join('; ')}` : null,
  }
}

/// The tickets a footer count opens: waiting ones whole; the rest are the
/// cards the plan carries in that state, so the list can be shorter than the
/// count (the board has more cards than a plan names).
export function listOf(plan: Plan, kind: CountKind, statuses: Statuses = statusesOf(plan)): { key: string; task: string; title: string; initiative: string | null }[] {
  const cardOf = new Map(plan.cards.map(c => [c.task, c]))
  const themeOf = (task: string) => plan.themes.find(t => t.state !== 'dropped' && t.tasks.includes(task))?.name ?? null
  const row = (task: string, key: string) => ({ key, task, title: cardOf.get(task)?.title || key, initiative: themeOf(task) })
  const waiting = waitingTickets(plan, statuses)
  if (kind === 'waiting') return waiting.map(t => row(t.task, t.key))
  const inWaiting = new Set(waiting.map(t => t.task))
  const want = kind === 'done' ? ['done'] : ['backlog', 'todo']
  return plan.cards.filter(c => want.includes(statuses.get(c.task) ?? '') && !(kind === 'notStarted' && inWaiting.has(c.task))).map(c => row(c.task, c.key))
}

// MARK: Needs You

/// One Needs You line: its title, what it asks, how long it has waited, and
/// the words a click leaves in the orchestrator's input.
export interface NeedsRow {
  id: string
  title: string
  detail: string
  since: number | null
  quote: string
}

/// `raw` as one line that does nothing when it reaches a composer: escape
/// sequences, control characters, line breaks and invisible format characters
/// removed, runs of space made one (the Mac's `AskAboutTask.oneLine`).
export function oneLine(raw: string): string {
  let out = ''
  const chars = [...raw]
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i].codePointAt(0) as number
    if (c === 0x1b) {
      const kind = chars[++i]
      if (kind === '[') {
        // CSI: parameters, then one final byte, 0x40 to 0x7E.
        while (++i < chars.length) {
          const n = chars[i].codePointAt(0) as number
          if (n >= 0x40 && n <= 0x7e) break
        }
      } else if (kind === ']') {
        // OSC: up to BEL, or ESC and a backslash.
        while (++i < chars.length) {
          const n = chars[i].codePointAt(0) as number
          if (n === 0x07) break
          if (n === 0x1b) {
            i++
            break
          }
        }
      }
    } else if ([0x09, 0x0a, 0x0d, 0x0b, 0x0c, 0x85, 0x2028, 0x2029].includes(c)) out += ' '
    else if (
      c <= 0x1f ||
      (c >= 0x7f && c <= 0x9f) ||
      c === 0xad ||
      (c >= 0x200b && c <= 0x200f) ||
      (c >= 0x202a && c <= 0x202e) ||
      (c >= 0x2060 && c <= 0x2064) ||
      (c >= 0x2066 && c <= 0x2069) ||
      c === 0xfeff
    ) {
      // dropped
    } else out += chars[i]
  }
  return out.split(' ').filter(Boolean).join(' ')
}

/// What a Needs You row leaves in the orchestrator's input: a reference for
/// the person to finish, never sent. "About ov-1 (“Site copy”): ".
export function quote(key: string | null, title: string): string {
  if (key) return `About ${oneLine(key)} (“${oneLine(title)}”): `
  return `About “${oneLine(title)}”: `
}

/// The shape of a Needs You item this needs.
export interface NeedsItem {
  id: string
  since: number | null
  question: string
  task: { key: string; title: string } | null
  terminal: { label: string } | null
}

/// The rows: each initiative asking the owner first, then the runner's list.
export function needsRows(plan: Plan | null, items: NeedsItem[]): NeedsRow[] {
  const asks = plan ? shownThemes(plan).filter(t => t.ask.trim() !== '') : []
  const rows: NeedsRow[] = asks.map(t => ({ id: `theme:${t.id}`, title: t.name, detail: t.ask.trim(), since: null, quote: quote(null, `${t.name}: ${t.ask.trim()}`) }))
  for (const item of items) {
    const title = item.task?.title || item.question
    const detail = item.task ? item.question : item.terminal?.label ?? ''
    rows.push({ id: item.id, title, detail, since: item.since, quote: quote(item.task?.key || null, title) })
  }
  return rows
}
