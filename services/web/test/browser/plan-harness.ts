// The work screen's Plan half, for plan-capture.mjs: the page's own markup,
// stylesheet and view code, over a plan written to the shape ov-867's C10f was
// drawn over (long real titles, a stuck builder, a reviewer with more waiting,
// a landing of four tickets), instead of a tunnel. The tab switch mirrors
// app.ts's showTab.

import { PlanBoard, type PlanBoardView } from '../../src/client/plan'
import { needsRows } from '../../src/client/plan-column'
import { renderPlan } from '../../src/client/plan-view'

const $ = (id: string) => document.getElementById(id) as HTMLElement
const NOW = 1_800_000_000_000
const MIN = 60_000
const task = (n: number) => `00000000-0000-0000-0000-${String(n).padStart(12, '0')}`
const key = (n: number) => `ov-${n}`
const ref = (n: number) => ({ task: task(n), key: key(n) })
const card = (n: number, title: string, status = 'In Progress') => ({ ...ref(n), title, status })
const theme = (n: number, name: string, goal: string, done: number, total: number, cards: number[], o: { next?: string; ask?: string; state?: string } = {}) => ({
  id: `theme-${n}`, name, outcome: goal, story: '', next: o.next ?? '', owner_ask: o.ask ?? '', state: o.state ?? 'active', ordinal: n,
  cards: cards.map(ref), counts: { backlog: total - done, todo: 0, needs_decision: 0, in_progress: 0, in_review: 0, done, cancelled: 0 },
})
const line = (n: number, stage: string, since: number, o: { stale?: boolean; landing?: number } = {}) => ({
  ...ref(n), title: '', status: 'In Progress', stage, stage_since: NOW - since * MIN, stale: o.stale ?? false, landing: o.landing ?? null, reason: '', plan_rank: null, work: [],
})
const agent = (id: string, role: string, started: number, doing: string, tickets: [number, string][] = [], o: { terminal?: string } = {}) => ({
  id, role, harness: 'claude', model: 'sonnet', title: '', doing_now: doing, terminal: o.terminal ?? null, working: true, started_at: NOW - started * MIN, last_at: NOW,
  tickets: tickets.map(([n, work]) => ({ ...ref(n), role: work, started_at: NOW - started * MIN })),
})

const wire = {
  now_ms: NOW,
  board_counts: { backlog: 22, todo: 10, needs_decision: 1, in_progress: 9, in_review: 15, done: 282, cancelled: 6 },
  themes: [
    theme(1, 'Web client', 'You and your teammates use Far Cooler fully from any browser', 20, 29, [869, 870, 871, 876], { next: 'a shared computer signs in with a phone' }),
    theme(2, 'Mobile experience', 'From the phone you check in on everything and make the calls only you can make', 16, 24, [880, 881, 882, 883, 884], { ask: 'the Play service account' }),
    theme(3, 'Team mode', 'Far Cooler works for teams that require PR review', 55, 60, [877, 890], { next: 'you try per-person access on real devices' }),
    theme(4, 'Engineering health', 'Main stays green and fast', 49, 51, [892], { next: 'confirm tonight’s nightly is green' }),
    theme(5, 'Plan v2', 'The plan stays current by itself and says more', 15, 17, [867, 868], { ask: 'page actions and promotions wait on your direction' }),
    theme(6, 'Trustworthy agent view', 'The agent view never freezes, never blanks, and matches the CLI', 48, 52, [885]),
    theme(7, 'Mac look and feel', 'The Mac app feels like it was made by Apple', 33, 34, [891]),
    theme(8, 'Marketing', 'Someone who has never seen Far Cooler understands it in a minute', 6, 8, []),
  ],
  cards: [
    card(869, 'An already-paired browser starts after an update'),
    card(870, 'The web is organized by workspace, like the apps'),
    card(871, 'A failed connection says why, and fails less', 'In Review'),
    card(876, 'Sign-in limits per visitor and account', 'In Review'),
    card(880, 'The Android app no longer crashes on a cold start after the Play update'),
    card(881, 'Chat scrolling stutters now and then'),
    card(882, 'The Task screen scrolls smoothly'),
    card(883, 'Holding a photo doesn’t hit its X'),
    card(884, 'People is reachable on iPhone and Android', 'In Review'),
    card(877, 'A revoked device says it was removed', 'In Review'),
    card(890, 'Step-up on a Mac that isn’t the runner', 'In Review'),
    card(891, 'A terminal doesn’t garble when the app comes back', 'In Review'),
    card(892, 'A late tick loses no rows on a fling', 'In Review'),
    card(885, 'Finished subagents don’t pile up at the end'),
    card(867, 'The plan reads like an assembly line'),
    card(868, 'Builder, reviewer and lander agents replace the old plan words'),
    card(894, 'Queued: the watch says what’s landing', 'To Do'),
  ],
  lines: [
    line(869, 'building', 30), line(870, 'building', 23), line(881, 'building', 2), line(882, 'building', 2), line(885, 'building', 23),
    line(880, 'building', 150, { stale: true }), line(883, 'building', 32), line(867, 'building', 32), line(868, 'building', 6),
    line(871, 'reviewing', 8), line(876, 'reviewing', 8), line(877, 'reviewing', 40), line(884, 'landing', 14, { landing: 120 }),
    line(891, 'landing', 14, { landing: 120 }), line(892, 'landing', 14, { landing: 120 }), line(890, 'landing', 14, { landing: 120 }),
    line(894, 'waiting', 5),
  ],
  landings: [{
    id: 'landing-120', number: 120, title: '', summary: '', state: 'gating', phase: 'gating_locally', phase_since: NOW - 14 * MIN, collecting: false,
    tickets: [884, 891, 892, 890].map(ref), lander: 'agent:i1',
  }],
  agents: [
    agent('pane:orchestrator', 'orchestrator', 240, '', [], { terminal: 'orch' }),
    agent('agent:b1', 'builder', 30, 'Finding why a paired browser sticks on Starting', [[869, 'build']]),
    agent('agent:b2', 'builder', 23, 'Grouping the web’s panes by workspace', [[870, 'build']]),
    agent('agent:b3', 'builder', 2, 'Reading the chat list’s scroll code', [[881, 'build'], [882, 'build']]),
    agent('agent:b4', 'builder', 23, 'Keeping finished subagents in their turns', [[885, 'build']]),
    agent('agent:b5', 'builder', 47, 'Rerunning the Android unit tests', [[880, 'fix']]),
    agent('agent:b6', 'builder', 32, 'Moving the photo’s X out of the hold area', [[883, 'build']]),
    agent('agent:b7', 'builder', 32, 'Drawing the plan around agents and their tickets', [[867, 'build']]),
    agent('agent:b8', 'builder', 6, 'Reading how the work is dispatched today', [[868, 'build']]),
    agent('agent:r1', 'reviewer', 8, 'Reading the diff for a failed web connection', [[871, 'review'], [876, 'review']]),
    agent('agent:i1', 'lander', 14, '', [[884, 'land'], [891, 'land'], [892, 'land'], [890, 'land']]),
  ],
  rulings: [],
}
const text = JSON.stringify(wire)
const refuse = (window as any).refuse === true
const needs = [
  { id: 'n1', since: NOW - 12 * MIN, question: 'Allow Bash to run the Android unit tests?', task: { key: key(880), title: 'The Android app no longer crashes on a cold start after the Play update' }, terminal: null },
]
const quoted: string[] = ((window as any).quoted = [])
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
      NOW + 30_000,
      {
        detail: c => quoted.push(`open ${c.key}`),
        quote: t => (quoted.push(t), true),
        openAgent: a => (quoted.push(`agent ${a.name}`), true),
        openList: k => quoted.push(`list ${k}`),
        edit: id => board.edit(id),
        type: d => board.type(d),
        save: () => void board.save(),
        cancel: () => board.cancel(),
        keep: id => void board.keep(id),
        keepAll: () => void board.keepAll(),
      },
      drawn,
      needsRows(view.plan, needs),
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
  board.edit('theme-1')
  board.type('Ship it before Friday')
  await board.save()
}
;(window as any).done = true
