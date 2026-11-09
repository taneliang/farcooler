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

/// The view's types are generated from crates/client (generated/wire-types.ts, ov-1035); an absent
/// optional field is left out of the JSON, so a field typed `T | undefined` is exactly that.
export type { Run, Fact, Initiative, PipeStage, Role, ColumnTicket, ColumnAgent, Group, Count, WaitingTicket, Column } from './generated/wire-types'
export type { PlanView as PlanViewModel } from './generated/wire-types'
import type { PlanView as PlanViewModel } from './generated/wire-types'

/// What the page knows beside the plan.
export interface PlanContext {
  /// The workspace's Needs You count.
  needsYou: number
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
    context: { needs_you: context.needsYou, statuses, titles, agents_supported: Array.isArray(plan?.agents), style: 'apple' },
  }
  try {
    const answer = JSON.parse(present(JSON.stringify(request)))
    return answer?.schema === 1 && answer.view ? (answer.view as PlanViewModel) : null
  } catch {
    return null
  }
}
