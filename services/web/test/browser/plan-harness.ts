// The work screen's Plan half, for plan-capture.mjs: the page's own markup,
// stylesheet and view code, over test/fixtures/plan.json with a few long rows
// added, instead of a tunnel. The tab switch mirrors app.ts's showTab.

import fixture from '../../../../test/fixtures/plan.json'
import { PlanBoard, type PlanBoardView } from '../../src/client/plan'
import { renderPlan } from '../../src/client/plan-view'

const $ = (id: string) => document.getElementById(id) as HTMLElement
const NOW = fixture.now_ms + 5 * 60_000
const wire: any = JSON.parse(JSON.stringify(fixture))
wire.lanes.push({
  ...wire.lanes[1],
  id: 'lane-build',
  name: 'billing-export',
  title: 'Billing: invoice PDF export with the long customer addresses folded properly',
  reason: 'Cards bil-7 and bil-9 share the renderer, so one agent builds both.',
  state: 'building',
  train: null,
  cards: [{ key: 'bil-7', slice: '', task: 'a' }, { key: 'bil-9', slice: '', task: 'b' }],
  agents: [{ agent_id: 'x', role: 'build', model: 'opus', started_at: 1, ended_at: null }],
  state_since: fixture.now_ms - 3 * 3_600_000,
})
wire.themes.push({
  ...wire.themes[0],
  id: 'theme-two',
  name: 'Team mode on the web with every member’s device',
  outcome: 'A whole team works with one orchestrator from a browser or a phone.',
  story: 'The conversation view is in review; the plan view is next, then the shared computer sign-in.',
  next: 'Plan view shows and edits live, with the runner’s refusals in plain words',
  owner_ask: 'Should members be able to drop a lane?',
  ordinal: 1,
})
const text = JSON.stringify(wire)
const refuse = (window as any).refuse === true
let drawn = ''
const board: PlanBoard = new PlanBoard({
  runner: 'studio',
  workspaces: async () => JSON.stringify([{ id: 'ws', name: 'Main', is_main: true }]),
  plan: async () => text,
  setNext: async () => {
    throw Object.assign(new Error('refused'), { code: 'not-yours', what: 'not_yours' })
  },
  keep: async () => {},
  keepAll: async () => 1,
  show: (view: PlanBoardView) => {
    drawn = renderPlan(
      $('plan'),
      view,
      NOW,
      {
        choose: id => board.choose(id),
        edit: id => board.edit(id),
        type: d => board.type(d),
        save: () => void board.save(),
        cancel: () => board.cancel(),
        keep: id => void board.keep(id),
        keepAll: () => void board.keepAll(),
      },
      drawn,
    )
  },
})

$('runner-name').textContent = 'studio'
$('who').textContent = 'owner@example.com'
$('work').hidden = false
$('status').hidden = true
$('plan').hidden = false
$('needs').hidden = true
$('pane').hidden = true
$('terminal-list').hidden = true
$('show-terminals').setAttribute('aria-pressed', 'false')
$('show-plan').setAttribute('aria-pressed', 'true')
await board.refresh()
if (refuse) {
  board.edit('theme-two')
  board.type('Ship it before Friday')
  await board.save()
  document.querySelector('.plan-notice')?.scrollIntoView({ block: 'center' })
}
;(window as any).done = true
