/// The plan tab's glue (ov-775, ov-875): a board over the connected runner,
/// drawn into the page and read again while the tab is in front. app.ts calls
/// this and nothing else of the plan.

import { type CardRef, type Plan, PLAN_POLL_MS, PlanBoard, type PlanBoardView } from './plan'
import type { CountKind, NeedsItem } from './plan-column'
import { type PlanPresenter, type PlanViewModel, presentPlan } from './plan-present'
import { renderPlan } from './plan-view'
import type { WebRunner } from 'fc-web'

export interface PlanTab {
  /// The tab came to the front (true) or went behind (false).
  show(front: boolean): void
  /// Another workspace's plan.
  choose(workspace: string): void
  /// Needs You in this workspace, for the column's own list.
  setNeeds(items: NeedsItem[]): void
  /// The plan as last read with its view, and a footer count's size, for
  /// that count's list.
  listing(kind: CountKind): { plan: Plan; view: PlanViewModel; count: number } | null
  /// The terminal of this workspace's orchestrator, when the plan names one.
  orchestrator(): string | null
  stop(): void
}

/// What the column's rows do in this page.
export interface PlanActions {
  /// A card's detail.
  openCard(card: CardRef): void
  /// Leave `text` in the orchestrator's input, unsent; false with none to put it in.
  quote(text: string): boolean
  /// Open an agent's chat; false when it has none.
  openAgent(agent: { id: string; name: string; terminal: string | null }): boolean
  openList(kind: CountKind): void
}

/// The orchestrator's pane: its own `terminal`, else the pane its id names
/// (`pane:<terminal id>`).
export function orchestratorOf(plan: Plan | null): string | null {
  const agent = plan?.agents.find(a => a.role === 'orchestrator')
  if (!agent) return null
  return agent.terminal ?? (agent.id.startsWith('pane:') ? agent.id.slice(5) : null)
}

/// `current` is the connection as it is now, or null when there is none.
export function startPlanTab(
  root: HTMLElement,
  runner: string,
  current: () => WebRunner | null,
  workspace: string | null,
  actions: PlanActions,
  /// The view model's presenter (wasm's `planView`).
  present: PlanPresenter,
  /// The workspace's workflow presentation, or null when it has none.
  presentation: (workspace: string) => Promise<unknown>,
): PlanTab {
  const live = (): WebRunner => {
    const r = current()
    if (!r) throw new Error('closed')
    return r
  }
  let drawn = ''
  let items: NeedsItem[] = []
  let last: PlanBoardView | null = null
  const board: PlanBoard = new PlanBoard({
    runner,
    workspace,
    workspaces: () => live().workspaces(),
    plan: workspace => live().plan(workspace),
    presentation,
    setNext: (theme, next) => live().setThemeNext(theme, next),
    keep: ruling => live().keepRuling(ruling),
    keepAll: workspace => live().keepAllRulings(workspace),
    show: view => {
      last = view
      drawn = renderPlan(
        root,
        view,
        Date.now(),
        {
          detail: actions.openCard,
          quote: actions.quote,
          openAgent: actions.openAgent,
          openList: actions.openList,
          edit: id => board.edit(id),
          type: draft => board.type(draft),
          save: () => void board.save(),
          cancel: () => board.cancel(),
          keep: id => void board.keep(id),
          keepAll: () => void board.keepAll(),
        },
        drawn,
        items,
        present,
      )
    },
  })
  let timer = 0
  return {
    show(front) {
      clearInterval(timer)
      if (!front) return
      void board.refresh()
      timer = window.setInterval(() => {
        if (document.visibilityState === 'visible') void board.refresh()
      }, PLAN_POLL_MS)
    },
    choose(id) {
      board.choose(id)
    },
    setNeeds(next) {
      if (JSON.stringify(next) === JSON.stringify(items)) return
      items = next
      if (last && !last.editing) board.redraw()
    },
    listing(kind) {
      if (!last?.plan) return null
      const view = presentPlan(present, last.raw, { needsYou: items.length, workflow: last.workflow })
      return view ? { plan: last.plan, view, count: view.column.footer.find(c => c.kind === kind)?.count ?? 0 } : null
    },
    orchestrator: () => orchestratorOf(last?.plan ?? null),
    stop() {
      clearInterval(timer)
      board.stop()
    },
  }
}
