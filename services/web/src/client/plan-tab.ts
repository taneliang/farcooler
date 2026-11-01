/// The plan tab's glue (ov-775, ov-875): a board over the connected runner,
/// drawn into the page and read again while the tab is in front. app.ts calls
/// this and nothing else of the plan.

import { type CardRef, type Plan, PLAN_POLL_MS, PlanBoard, type PlanBoardView } from './plan'
import { type Agent, type CountKind, type NeedsItem, needsRows, type NeedsRow } from './plan-column'
import { renderPlan } from './plan-view'
import type { WebRunner } from 'fc-web'

export interface PlanTab {
  /// The tab came to the front (true) or went behind (false).
  show(front: boolean): void
  /// Another workspace's plan.
  choose(workspace: string): void
  /// Needs You in this workspace, for the column's own list.
  setNeeds(items: NeedsItem[]): void
  /// The plan as last read, for a footer count's list.
  current(): Plan | null
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
  openAgent(agent: Agent): boolean
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
export function startPlanTab(root: HTMLElement, runner: string, current: () => WebRunner | null, workspace: string | null, actions: PlanActions): PlanTab {
  const live = (): WebRunner => {
    const r = current()
    if (!r) throw new Error('closed')
    return r
  }
  let drawn = ''
  let items: NeedsItem[] = []
  let last: PlanBoardView | null = null
  const rows = (view: PlanBoardView): NeedsRow[] => needsRows(view.plan, items)
  const board: PlanBoard = new PlanBoard({
    runner,
    workspace,
    workspaces: () => live().workspaces(),
    plan: workspace => live().plan(workspace),
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
        rows(view),
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
    current: () => last?.plan ?? null,
    orchestrator: () => orchestratorOf(last?.plan ?? null),
    stop() {
      clearInterval(timer)
      board.stop()
    },
  }
}
