// plan-render-check.mjs's page: the plan view code over the golden every
// platform decodes (test/fixtures/plan-view-apple.json), through a stub
// presenter that answers with the golden's own answer, so the check says the
// renderer draws the view's words and nothing else.

import original from '../../../../test/fixtures/plan-view-apple.json'
import focused from '../../../../test/fixtures/plan-pipeline.json'

const golden = (window as any).focused ? focused : original
import queue from '../../../../test/fixtures/landing-queue.json'
import { startPlanTab } from '../../src/client/plan-tab'
import type { WebRunner } from 'fc-web'
import { parsePlan } from '../../src/client/plan'
import type { PlanPresenter } from '../../src/client/plan-present'
import { renderPlan } from '../../src/client/plan-view'
import { harnessShell, runnerNamed } from './harness-shell'

harnessShell({ runner: runnerNamed('Mac mini'), page: 'plan' })

const present: PlanPresenter = () => JSON.stringify(golden.answer)
const plan = parsePlan(JSON.stringify(golden.request.plan))!
const none = () => {}
const checkQuote = (window as any).checkQuote === true
renderPlan(
  document.getElementById('plan') as HTMLElement,
  { state: 'ready', plan, raw: golden.request.plan, runner: 'Mac mini', workspaces: [], workspace: null, editing: null, busy: new Set(), notices: new Map(), banner: '' },
  golden.answer.view.now,
  { detail: none, quote: async () => !checkQuote, openAgent: () => true, openList: none, edit: none, type: none, save: none, cancel: none, keep: none, keepAll: none, retry: none },
  '',
  checkQuote ? [{ id: 'quote-test', question: 'Quote delivery check', since: null, task: null, terminal: null }] : [],
  present,
)
for (const id of ['work', 'plan']) document.getElementById(id)!.hidden = false
for (const id of ['status', 'needs', 'pane', 'terminal-list']) document.getElementById(id)!.hidden = true
;(window as any).golden = golden.answer.view
// A queue without themes, line tickets or pipeline agents must still reach the
// current Plan screen. Its complete words come from the Rust queue golden.
const queueRoot = document.createElement('div')
queueRoot.id = 'landing-queue'
queueRoot.className = 'plan'
document.getElementById('plan')!.after(queueRoot)
document.getElementById('work')!.hidden = false
document.getElementById('status')!.hidden = true
document.getElementById('pane')!.hidden = true
document.getElementById('terminal-list')!.hidden = true
// The runner always sends these arrays; the Rust input golden can omit them.
const queuePlan = { ...queue.request.plan, themes: [], lines: [] }
const queueView = {
  ...golden.answer.view,
  column: { ...golden.answer.view.column, facts: [], pipeline: [], roster: [], initiatives: [], quietInitiatives: [], initiativeCount: 0, footer: [] },
  sections: [{ kind: 'landing', title: 'Landings', count: queue.expected.length }],
  landingGroups: queue.expected,
  landingGroupHeading: 'Landings',
}
renderPlan(
  queueRoot,
  { state: 'ready', plan: parsePlan(JSON.stringify(queuePlan))!, raw: queuePlan, runner: 'Mac mini', workspaces: [], workspace: null, editing: null, busy: new Set(), notices: new Map(), banner: '' },
  queue.request.plan.now_ms,
  { detail: none, quote: () => true, openAgent: () => true, openList: none, edit: none, type: none, save: none, cancel: none, keep: none, keepAll: none, retry: none },
  '', [], request => {
    ;(window as any).queueRequest = JSON.parse(request)
    return JSON.stringify({ schema: 1, view: queueView })
  },
)
;(window as any).queueGolden = queue

// Exercise the Plan tab's retry with its polling timer stopped.
if ((window as any).failWorkflowRead) {
  const root = document.getElementById('plan')!
  let reads = 0
  const runner = {
    workspaces: async () => JSON.stringify([{ id: 'ws', is_main: true }]),
    boardOrderLabel: () => undefined,
    plan: async () => {
      if (++reads === 1) throw Object.assign(new Error('refused'), { code: 'operation-failed', what: 'response_too_large' })
      return JSON.stringify(queuePlan)
    },
  } as Partial<WebRunner> as WebRunner
  const tab = startPlanTab(root, 'Mac mini', () => runner, null,
    { openCard: none, quote: () => true, openAgent: () => true, openList: none },
    request => JSON.stringify(JSON.parse(request).engine_words
      ? { schema: 1, words: { tryAgain: 'Try Again' } }
      : { schema: 1, view: queueView }))
  tab.show(true)
  tab.show(false)
}
;(window as any).done = true
