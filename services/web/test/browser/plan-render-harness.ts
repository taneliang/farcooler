// plan-render-check.mjs's page: the plan view code over the golden every
// platform decodes (test/fixtures/plan-view-apple.json), through a stub
// presenter that answers with the golden's own answer, so the check says the
// renderer draws the view's words and nothing else.

import original from '../../../../test/fixtures/plan-view-apple.json'
import focused from '../../../../test/fixtures/plan-pipeline.json'

const golden = (window as any).focused ? focused : original
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
for (const id of ['work', 'plan']) document.getElementById(id)!.hidden = false
for (const id of ['status', 'needs', 'pane', 'terminal-list']) document.getElementById(id)!.hidden = true
document.getElementById('show-terminals')!.setAttribute('aria-pressed', 'false')
document.getElementById('show-plan')!.setAttribute('aria-pressed', 'true')
;(window as any).golden = golden.answer.view
;(window as any).done = true
