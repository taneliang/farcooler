/// The plan's view model on the page (ov-1016, ruling R-112): the plan column
/// (facts, initiatives, the agent pipeline, the footer) comes
/// from crates/client's `plan_view` through wasm's `planView`. The browser used
/// to sort the plan itself (plan-column.ts) and say its own words; now it
/// draws what the view says, in its own layout (plan-view.ts).
///
/// The presenter is injected so a test, which cannot load wasm, hands in a
/// stub that answers with the golden (test/fixtures/plan-view-apple.json).
/// Nothing here touches the page.

/// A presenter: a request as JSON in, the answer as JSON out. The page passes
/// wasm's `planView`.
export type PlanPresenter = (request: string) => string

export interface Run {
  text: string
  strong: boolean
}

export interface Fact {
  kind: 'needs_you' | 'stuck' | 'next_landing' | 'done'
  runs: Run[]
  /// Drawn in amber: it needs the owner, or something is stuck.
  attention: boolean
  text: string
}

export interface Initiative {
  id: string
  name: string
  /// The outcome the theme was written for; empty when it has none.
  goal: string
  progress: string
  /// How full its meter is, 0 to 1.
  fraction: number
  /// "Next: …", or "Waiting on you: …" (`asks_owner`).
  next: string | null
  asks_owner: boolean
}

export type PipeStage = 'stuck' | 'building' | 'reviewing' | 'landing'
export type Role = 'builder' | 'reviewer' | 'lander' | 'agent'

export interface ColumnTicket {
  task: string
  key: string
  title: string
  initiative: string | null
}

export interface ColumnAgent {
  id: string
  role: Role
  /// "Builder 2".
  name: string
  doing: string
  /// `doing` says why it's stuck: drawn in amber.
  doing_is_trouble: boolean
  /// "47 min"; null when nothing says.
  time: string | null
  terminal: string | null
  /// Its first ticket, for its page when there's no pane to open.
  ticket: string | null
  tickets: ColumnTicket[]
  working: boolean
  /// Tickets in this stage nobody is on, said under the last card.
  more: number
}

export interface Group {
  stage: PipeStage
  title: string
  agents: ColumnAgent[]
  /// Tickets in this stage with no agent on them.
  unattended: ColumnTicket[]
  /// The tickets in this stage, each once.
  count: number
  /// "12 more waiting for review".
  unattended_words: string | null
}

export interface Count {
  kind: 'waiting' | 'not_started' | 'done'
  count: number
  /// "10 tickets waiting for a builder", "22 not started", "796 done".
  words: string
}

export interface WaitingTicket {
  task: string
  key: string
  title: string
}

/// The plan column (ov-867): the facts line, the initiatives, the pipeline of
/// agents with their tickets, and the footer counts.
export interface Column {
  facts: Fact[]
  initiatives: Initiative[]
  quiet_initiatives: Initiative[]
  /// "Nothing moving: Mac look and feel, 33 of 34".
  quiet_line: string | null
  initiative_count: number
  pipeline: Group[]
  /// "10 agents", "1 agent".
  agent_words: string
  footer: Count[]
  /// The tickets waiting for a builder, each once, in the plan's order.
  waiting: WaitingTicket[]
}

export interface PlanViewModel {
  schema: number
  now: number
  column: Column
}

/// What the page knows beside the plan.
export interface PlanContext {
  /// The workspace's Needs You count.
  needsYou: number
  /// The workspace's workflow presentation (`workflow.snapshot`), or null.
  workflow: unknown
}

/// The view of `plan` (the runner's `plan.get`, parsed), or null when the
/// presenter couldn't read it. The board's statuses are the plan's own card
/// words by task; the view accepts the board's word or the store's.
export function presentPlan(present: PlanPresenter, plan: any, context: PlanContext): PlanViewModel | null {
  const statuses: Record<string, string> = {}
  for (const list of [plan?.lines, plan?.cards]) {
    if (!Array.isArray(list)) continue
    for (const c of list) if (c && typeof c.task === 'string' && typeof c.status === 'string' && c.status) statuses[c.task] = c.status
  }
  // The board's card titles by task, for a ticket the plan names no title for.
  const titles: Record<string, string> = {}
  for (const list of [plan?.cards, plan?.lines]) {
    if (!Array.isArray(list)) continue
    for (const c of list) if (c && typeof c.task === 'string' && typeof c.title === 'string' && c.title) titles[c.task] ??= c.title
  }
  const request = {
    schema: 1,
    plan,
    context: { needs_you: context.needsYou, statuses, titles, agents_supported: Array.isArray(plan?.agents), style: 'web' },
    workflow: context.workflow ?? null,
  }
  try {
    const answer = JSON.parse(present(JSON.stringify(request)))
    return answer?.schema === 1 && answer.view ? (answer.view as PlanViewModel) : null
  } catch {
    return null
  }
}
