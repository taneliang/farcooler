/// The plan tab's glue (ov-775): a board over the connected runner, drawn into
/// the page and read again while the tab is in front. app.ts calls this and
/// nothing else of the plan.

import { PLAN_POLL_MS, PlanBoard } from './plan'
import { renderPlan } from './plan-view'
import type { WebRunner } from 'fc-web'

export interface PlanTab {
  /// The tab came to the front (true) or went behind (false).
  show(front: boolean): void
  stop(): void
}

/// `current` is the connection as it is now, or null when there is none.
export function startPlanTab(root: HTMLElement, runner: string, current: () => WebRunner | null): PlanTab {
  const live = (): WebRunner => {
    const r = current()
    if (!r) throw new Error('closed')
    return r
  }
  let drawn = ''
  const board: PlanBoard = new PlanBoard({
    runner,
    workspaces: () => live().workspaces(),
    plan: workspace => live().plan(workspace),
    setNext: (theme, next) => live().setThemeNext(theme, next),
    keep: ruling => live().keepRuling(ruling),
    keepAll: workspace => live().keepAllRulings(workspace),
    show: view => {
      drawn = renderPlan(
        root,
        view,
        Date.now(),
        {
          choose: id => board.choose(id),
          edit: id => board.edit(id),
          type: draft => board.type(draft),
          save: () => void board.save(),
          cancel: () => board.cancel(),
          keep: id => void board.keep(id),
          keepAll: () => void board.keepAll(),
        },
        drawn,
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
    stop() {
      clearInterval(timer)
      board.stop()
    },
  }
}
