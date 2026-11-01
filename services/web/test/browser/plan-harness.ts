// The work screen's Plan half, for plan-capture.mjs: the page's own markup,
// stylesheet and view code, over a plan written to the shape ov-867's C10f was
// drawn over (long real titles, a stuck builder, a reviewer with more waiting,
// a train landing four tickets), instead of a tunnel. The tab switch mirrors
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
const lane = (n: number, name: string, state: string, cards: number[], since: number, o: { train?: string; stale?: boolean } = {}) => ({
  id: `lane-${n}`, name, title: '', state, state_since: NOW - since * MIN, stale: o.stale ?? false, train: o.train ?? null, cards: cards.map(c => ({ ...ref(c), slice: '' })), agents: [],
})
const agent = (id: string, role: string, started: number, label: string, o: { lane?: number; train?: number; terminal?: string } = {}) => ({
  id, role, harness: 'claude', model: 'sonnet', label, lane: o.lane ? { id: `lane-${o.lane}`, name: `lane-${o.lane}` } : null,
  train: o.train ? { id: `train-${o.train}`, title: `Train ${o.train}` } : null, card: null, terminal: o.terminal ?? null, working: true, started_at: NOW - started * MIN, last_at: NOW,
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
    card(868, 'Builder, reviewer and lander agents replace lanes'),
    card(894, 'Queued: the watch says what’s landing', 'To Do'),
  ],
  lanes: [
    lane(1, 'web-paired', 'building', [869], 30), lane(2, 'web-workspaces', 'building', [870], 23), lane(3, 'ios-scroll', 'building', [881, 882], 2),
    lane(4, 'agent-view', 'building', [885], 23), lane(5, 'android-crash', 'fixing', [880], 150, { stale: true }), lane(6, 'ios-photo', 'building', [883], 32),
    lane(7, 'plan-c10', 'building', [867], 32), lane(8, 'agents-model', 'building', [868], 6), lane(9, 'web-errors', 'review', [871, 876], 8),
    lane(10, 'team-revoke', 'review', [877], 40), lane(11, 'people', 'landing', [884], 14, { train: 'train-120' }),
    lane(12, 'mac-terminal', 'landing', [891], 14, { train: 'train-120' }), lane(13, 'fling', 'landing', [892], 14, { train: 'train-120' }),
    lane(14, 'step-up', 'landing', [890], 14, { train: 'train-120' }), lane(15, 'watch', 'queued', [894], 5),
  ],
  trains: [{ id: 'train-120', name: 'train-120', title: 'Train 120', summary: '', state: 'gating', phase: 'gating_locally', phase_since: NOW - 14 * MIN, lanes: [11, 12, 13, 14].map(n => ({ lane: `lane-${n}`, name: `lane-${n}` })), agent: null }],
  agents: [
    agent('pane:orchestrator', 'orchestrator', 240, '', { terminal: 'orch' }),
    agent('agent:b1', 'build', 30, 'Finding why a paired browser sticks on Starting', { lane: 1 }),
    agent('agent:b2', 'build', 23, 'Grouping the web’s panes by workspace', { lane: 2 }),
    agent('agent:b3', 'build', 2, 'Reading the chat list’s scroll code', { lane: 3 }),
    agent('agent:b4', 'build', 23, 'Keeping finished subagents in their turns', { lane: 4 }),
    agent('agent:b5', 'fix', 47, 'Rerunning the Android unit tests', { lane: 5 }),
    agent('agent:b6', 'build', 32, 'Moving the photo’s X out of the hold area', { lane: 6 }),
    agent('agent:b7', 'build', 32, 'Drawing the plan around agents and their tickets', { lane: 7 }),
    agent('agent:b8', 'build', 6, 'Reading how lanes are dispatched today', { lane: 8 }),
    agent('agent:r1', 'review', 8, 'Reading the diff for a failed web connection', { lane: 9 }),
    agent('agent:i1', 'integrator', 14, '', { train: 120 }),
  ],
  order: [], rulings: [], lane_is_train: [],
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
