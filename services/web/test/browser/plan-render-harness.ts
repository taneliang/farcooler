// plan-render-check.mjs's page: the plan view code over the golden every
// platform decodes (test/fixtures/plan-view-apple.json), through a stub
// presenter that answers with the golden's own answer, so the check says the
// renderer draws the view's words and nothing else.

import golden from '../../../../test/fixtures/plan-view-apple.json'
import { parsePlan } from '../../src/client/plan'
import type { PlanPresenter } from '../../src/client/plan-present'
import { renderPlan } from '../../src/client/plan-view'

const present: PlanPresenter = () => JSON.stringify(golden.answer)
const plan = parsePlan(JSON.stringify(golden.request.plan))!
const none = () => {}
renderPlan(
  document.getElementById('plan') as HTMLElement,
  { state: 'ready', plan, raw: golden.request.plan, runner: 'Mac mini', workspaces: [], workspace: null, editing: null, busy: new Set(), notices: new Map(), banner: '' },
  golden.answer.view.now,
  { detail: none, quote: () => true, openAgent: () => true, openList: none, edit: none, type: none, save: none, cancel: none, keep: none, keepAll: none },
  '',
  [],
  present,
)
;(window as any).golden = golden.answer.view
;(window as any).done = true
