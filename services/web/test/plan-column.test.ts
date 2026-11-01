// The plan column (src/client/plan-column.ts, ov-875): the same answers the
// Mac's AgentKit `PlanColumn` gives for the same plan (its tests are
// PlanSectionsColumnTests): the facts line, Initiatives, the pipeline of
// agents with their tickets under them, each ticket on one agent, and the
// footer counts. The runner's plan is written by hand, in its own wire shape.

import { describe, expect, it } from 'vitest'

import fixturePlan from '../../../test/fixtures/plan.json'
import { type Plan, parsePlan } from '../src/client/plan'
import { columnOf, needsRows, oneLine, quote, span, statusesOf } from '../src/client/plan-column'

const NOW = 1_800_000_000_000
const MIN = 60_000

const text = (c: ReturnType<typeof columnOf>['facts']) => c.map(f => f.runs.map(r => r.text).join(''))

const agent = (
  id: string,
  role: string,
  o: { title?: string; doing?: string; terminal?: string; started: number; tickets?: [string, string][]; working?: boolean },
) => ({
  id,
  role,
  harness: 'claude',
  model: 'sonnet',
  title: o.title ?? '',
  doing_now: o.doing ?? '',
  terminal: o.terminal ?? null,
  working: o.working ?? true,
  started_at: NOW - o.started * MIN,
  last_at: NOW,
  tickets: (o.tickets ?? []).map(([t, work]) => ({ task: t, key: `ov-${t.slice(1)}`, role: work, started_at: NOW - o.started * MIN })),
})

const line = (task: string, stage: string, o: { since?: number; landing?: number; stale?: boolean; title?: string } = {}) => ({
  task,
  key: `ov-${task.slice(1)}`,
  title: o.title ?? '',
  status: 'In Progress',
  stage,
  stage_since: NOW - (o.since ?? 5) * MIN,
  stale: o.stale ?? false,
  landing: o.landing ?? null,
  reason: '',
  plan_rank: null,
  work: [],
})

const theme = (name: string, cards: string[], ordinal: number, o: { done?: number; total?: number; next?: string; ask?: string; state?: string } = {}) => ({
  id: `theme-${name}`,
  name,
  outcome: '',
  story: '',
  next: o.next ?? '',
  owner_ask: o.ask ?? '',
  state: o.state ?? 'active',
  ordinal,
  cards: cards.map(t => ({ task: t, key: `ov-${t.slice(1)}` })),
  counts: { backlog: (o.total ?? 1) - (o.done ?? 0), todo: 0, needs_decision: 0, in_progress: 0, in_review: 0, done: o.done ?? 0, cancelled: 0 },
})

const landing = (number: number, phase: string, tickets: string[], o: { phaseSince?: number; lander?: string } = {}) => ({
  id: `landing-${number}`,
  number,
  title: `Landing ${number}`,
  summary: '',
  state: phase === 'ci_failed' ? 'red' : 'gating',
  phase,
  phase_since: o.phaseSince ?? NOW - 14 * MIN,
  collecting: false,
  tickets: tickets.map(t => ({ task: t, key: `ov-${t.slice(1)}` })),
  lander: o.lander ?? null,
})

const card = (task: string, title: string, status = 'In Progress') => ({ task, key: `ov-${task.slice(1)}`, title, status })

function plan(o: { themes?: any[]; lines?: any[]; landings?: any[]; agents?: any[]; cards?: any[]; counts?: any }): Plan {
  const wire: any = { now_ms: NOW, themes: o.themes ?? [], lines: o.lines ?? [], cards: o.cards ?? [], landings: o.landings ?? [], agents: o.agents ?? [] }
  if (o.counts) wire.board_counts = o.counts
  return parsePlan(JSON.stringify(wire))!
}

describe('the pipeline', () => {
  it('gives each agent one card in the stage its work is, with its tickets under it and each ticket’s own initiative', () => {
    const p = plan({
      themes: [theme('Web client', ['t1'], 1), theme('Mobile experience', ['t2', 't3'], 2)],
      lines: [line('t1', 'building'), line('t2', 'building'), line('t3', 'reviewing')],
      agents: [
        agent('orchestrator', 'orchestrator', { started: 300 }),
        agent('agent:b', 'builder', { doing: 'Reading the chat list’s scroll code', terminal: 'term-b', started: 30, tickets: [['t1', 'build'], ['t2', 'build']] }),
        agent('agent:r', 'reviewer', { started: 8, tickets: [['t3', 'review']] }),
      ],
      cards: [card('t1', 'A paired browser starts after an update'), card('t2', 'Chat scrolling stutters')],
    })
    const c = columnOf(p)
    expect(c.pipeline.map(g => g.stage)).toEqual(['building', 'reviewing'])
    const builder = c.pipeline[0].agents[0]
    expect([builder.name, builder.role, builder.terminal, builder.doing, builder.elapsed]).toEqual(['Builder', 'builder', 'term-b', 'Reading the chat list’s scroll code', 30 * MIN])
    expect(builder.tickets.map(t => t.title)).toEqual(['A paired browser starts after an update', 'Chat scrolling stutters'])
    expect(builder.tickets.map(t => t.initiative)).toEqual(['Web client', 'Mobile experience'])
    const reviewer = c.pipeline[1].agents[0]
    expect(reviewer.name).toBe('Reviewer')
    expect(reviewer.tickets.map(t => t.key)).toEqual(['ov-3'])
    expect(c.agentWords).toBe('2 agents')
  })

  it('says an agent’s own words for what it does, else its title, else whether it works', () => {
    const p = plan({
      agents: [
        agent('agent:a', 'builder', { doing: '2/5 · Reviewing the plan', title: 'Build the glance', started: 9 }),
        agent('agent:b', 'builder', { title: 'Build the glance', started: 8 }),
        agent('agent:c', 'builder', { started: 7 }),
        agent('agent:d', 'builder', { started: 6, working: false }),
      ],
    })
    expect(columnOf(p).pipeline[0].agents.map(a => a.doing)).toEqual(['2/5 · Reviewing the plan', 'Build the glance', 'Working', 'Idle'])
  })

  it('numbers several agents of one role in the order they started', () => {
    const p = plan({
      lines: [line('t1', 'building'), line('t2', 'building')],
      agents: [agent('agent:late', 'builder', { started: 5, tickets: [['t1', 'build']] }), agent('agent:early', 'builder', { started: 40, tickets: [['t2', 'build']] })],
    })
    expect(columnOf(p).pipeline[0].agents.map(a => `${a.id}=${a.name}`).sort()).toEqual(['agent:early=Builder 1', 'agent:late=Builder 2'])
  })

  it('puts a stale ticket’s agent in Stuck, saying why, and counts it on the facts line with the longest time', () => {
    const p = plan({
      lines: [line('t1', 'building', { since: 150, stale: true }), line('t2', 'building')],
      agents: [
        agent('agent:s', 'builder', { doing: 'Running the tests', started: 47, tickets: [['t1', 'build']] }),
        agent('agent:ok', 'builder', { started: 3, tickets: [['t2', 'build']] }),
      ],
    })
    const c = columnOf(p, 2)
    expect(c.pipeline[0].stage).toBe('stuck')
    expect(c.pipeline[0].agents.map(a => a.id)).toEqual(['agent:s'])
    expect(c.pipeline[0].agents[0].doing).toBe('No move in 2 h')
    expect(c.pipeline[0].agents[0].doingIsTrouble).toBe(true)
    expect(text(c.facts)).toEqual(['2 need you', '1 stuck, 47 min'])
    expect(c.facts.find(f => f.kind === 'stuck')?.attention).toBe(true)
  })

  it('puts a ticket that needs the owner in Stuck, with no agent invented for it', () => {
    const c = columnOf(plan({ lines: [line('t1', 'needs_you')] }))
    expect(c.pipeline.map(g => g.stage)).toEqual(['stuck'])
    expect([c.pipeline[0].agents, c.pipeline[0].unattended.map(t => t.task), c.pipeline[0].unattendedWords]).toEqual([[], ['t1'], '1 ticket with no agent on it'])
  })

  it('says tickets nobody is on under their stage, never giving them an invented agent', () => {
    const p = plan({
      lines: [line('t1', 'reviewing'), line('t2', 'reviewing'), line('t3', 'reviewing'), line('t4', 'building')],
      agents: [agent('agent:r', 'reviewer', { started: 8, tickets: [['t1', 'review']] })],
    })
    const c = columnOf(p)
    const reviewing = c.pipeline.find(g => g.stage === 'reviewing')!
    expect([reviewing.count, reviewing.unattended.length]).toEqual([3, 2])
    expect(reviewing.unattendedWords).toBe('2 more waiting for review')
    expect(reviewing.agents[reviewing.agents.length - 1].more).toBe(2)
    const building = c.pipeline.find(g => g.stage === 'building')!
    expect(building.agents).toEqual([])
    expect(building.unattendedWords).toBe('1 ticket with no builder running')
    expect(c.agentCount).toBe(1)
  })

  it('leaves tickets that wait for a builder, landed ones and dropped ones out of the pipeline', () => {
    const c = columnOf(plan({ lines: [line('t1', 'waiting'), line('t2', 'landed'), line('t3', 'dropped')] }))
    expect(c.pipeline).toEqual([])
  })

  // The ticket on two agents is the failure this guards: a ticket a builder
  // and its lander both list would be counted twice.
  it('puts each ticket under exactly one agent, though several list it: its lander first, then the first to claim it', () => {
    const p = plan({
      lines: [line('t1', 'landing', { landing: 9 }), line('t2', 'building')],
      landings: [landing(9, 'gating_locally', ['t1'], { lander: 'agent:i' })],
      agents: [
        agent('agent:b1', 'builder', { started: 9, tickets: [['t1', 'build']] }),
        agent('agent:i', 'lander', { started: 20, tickets: [['t1', 'land']] }),
        agent('agent:b2', 'builder', { started: 8, tickets: [['t2', 'build']] }),
        agent('agent:b3', 'builder', { started: 7, tickets: [['t2', 'build']] }),
      ],
    })
    const c = columnOf(p)
    const all = c.pipeline.flatMap(g => [...g.agents.flatMap(a => a.tickets.map(t => t.task)), ...g.unattended.map(t => t.task)])
    expect(all.sort()).toEqual(['t1', 't2'])
    const by = (id: string) => c.pipeline.flatMap(g => g.agents).find(a => a.id === id)!
    expect(by('agent:i').tickets.map(t => t.task)).toEqual(['t1'])
    expect(by('agent:b1').tickets).toEqual([])
    expect(by('agent:b2').tickets.map(t => t.task)).toEqual(['t2'])
    expect(by('agent:b3').tickets).toEqual([])
    expect(c.pipeline.reduce((n, g) => n + g.count, 0)).toBe(2)
  })

  it('has an empty pipeline, no initiatives, no footer and no facts for an empty plan', () => {
    const c = columnOf(plan({}))
    expect(c.pipeline).toEqual([])
    expect([c.initiatives, c.quiet, c.footer, c.facts]).toEqual([[], [], [], []])
    expect(c.quietLine).toBeNull()
  })

  it('makes a live landing’s lander the lander over its tickets, and names the next landing on the facts line', () => {
    const p = plan({
      lines: [line('t1', 'landing', { landing: 9 }), line('t2', 'landing', { landing: 9 }), line('t3', 'landing', { landing: 9 }), line('t4', 'building')],
      landings: [landing(9, 'gating_locally', ['t1', 't2', 't3'], { lander: 'agent:i' })],
      agents: [agent('agent:i', 'lander', { started: 20, tickets: [['t1', 'land'], ['t2', 'land'], ['t3', 'land']] })],
      counts: { backlog: 20, todo: 2, needs_decision: 0, in_progress: 5, in_review: 3, done: 282, cancelled: 4 },
    })
    const c = columnOf(p)
    const lander = c.pipeline.find(g => g.stage === 'landing')!.agents[0]
    expect([lander.role, lander.name, lander.doing]).toEqual(['lander', 'Lander', 'Checking here'])
    expect(lander.tickets.map(t => t.task)).toEqual(['t1', 't2', 't3'])
    expect(c.pipeline.find(g => g.stage === 'building')!.unattended.map(t => t.task)).toEqual(['t4'])
    expect(text(c.facts)).toEqual(['Next landing: 3 tickets, 14 min', '282 of 312 done'])
  })

  it('finds a lander by its tickets’ landing when the landing names none', () => {
    const p = plan({
      lines: [line('t1', 'landing', { landing: 9 })],
      landings: [landing(9, 'waiting_for_ci', ['t1'])],
      agents: [agent('agent:i', 'lander', { started: 20, tickets: [['t1', 'land']] })],
    })
    expect(columnOf(p).pipeline[0].agents[0].doing).toBe('Checking on CI')
  })

  it('puts the lander of a landing whose CI failed in Stuck, and doesn’t call it the next landing', () => {
    const p = plan({
      lines: [line('t1', 'landing', { landing: 9 })],
      landings: [landing(9, 'ci_failed', ['t1'], { phaseSince: NOW - 5 * MIN, lander: 'agent:i' })],
      agents: [agent('agent:i', 'lander', { doing: 'Landing six', started: 20, tickets: [['t1', 'land']] })],
    })
    const c = columnOf(p)
    expect(c.pipeline.map(g => g.stage)).toEqual(['stuck'])
    expect([c.pipeline[0].agents[0].doing, c.pipeline[0].agents[0].doingIsTrouble]).toEqual(['CI failed', true])
    expect(c.facts.some(f => f.kind === 'nextLanding')).toBe(false)
  })
})

describe('initiatives', () => {
  it('lists those in motion or asking the owner, folds the rest into one quiet line, and drops done ones', () => {
    const p = plan({
      themes: [
        theme('Web client', ['t1'], 1, { done: 20, total: 29, next: 'a shared computer signs in' }),
        theme('Mobile', ['t9'], 2, { done: 16, total: 24, ask: 'the Play service account' }),
        theme('Mac look and feel', ['t8'], 3, { done: 33, total: 34 }),
        theme('Paused work', ['t7'], 4, { done: 1, total: 4, state: 'paused' }),
        theme('Release', [], 5, { done: 4, total: 4 }),
        theme('Old', [], 6, { done: 1, total: 2, state: 'done' }),
      ],
      lines: [line('t1', 'building'), line('t7', 'building')],
      agents: [agent('agent:b', 'builder', { started: 5, tickets: [['t1', 'build']] }), agent('agent:p', 'builder', { started: 5, tickets: [['t7', 'build']] })],
    })
    const c = columnOf(p)
    expect(c.initiatives.map(i => i.name)).toEqual(['Web client', 'Mobile'])
    expect([c.initiatives[0].next, c.initiatives[0].asksOwner]).toEqual(['Next: a shared computer signs in', false])
    expect([c.initiatives[1].next, c.initiatives[1].asksOwner]).toEqual(['Waiting on you: the Play service account', true])
    expect(c.quietLine).toBe('Nothing moving: Mac look and feel, 33 of 34; Paused work, 1 of 4')
    expect(c.initiativeCount).toBe(4)
  })
})

describe('the footer', () => {
  it('counts what waits for a builder, what hasn’t started and what is done, each ticket once', () => {
    const p = plan({
      lines: [line('t1', 'waiting'), line('t2', 'waiting'), line('t2', 'waiting'), line('t3', 'waiting')],
      cards: [card('t1', 'One', 'To Do'), card('t2', 'Two', 'To Do'), card('t3', 'Three', 'Backlog')],
      counts: { backlog: 20, todo: 5, needs_decision: 0, in_progress: 0, in_review: 0, done: 796, cancelled: 0 },
    })
    expect(columnOf(p).footer.map(f => f.words)).toEqual(['3 tickets waiting for a builder', '22 not started', '796 done'])
  })

  it('leaves a cancelled ticket that waits for a builder out, and draws Needs You in amber above zero', () => {
    const p = plan({ lines: [line('t1', 'waiting'), line('t2', 'waiting')], cards: [card('t1', 'One', 'To Do'), card('t2', 'Two', 'Cancelled')] })
    const c = columnOf(p, 1)
    expect(c.footer.find(f => f.kind === 'waiting')?.count).toBe(1)
    expect(c.facts.find(f => f.kind === 'needsYou')?.attention).toBe(true)
    // "Canceled" is the same status in the other spelling.
    expect(statusesOf(plan({ cards: [card('t1', 'One', 'Canceled')] })).get('t1')).toBe('cancelled')
  })

  it('leaves a cancelled ticket out of its agent as well', () => {
    const p = plan({
      lines: [line('t1', 'building'), line('t2', 'building')],
      agents: [agent('agent:b', 'builder', { started: 5, tickets: [['t1', 'build'], ['t2', 'build']] })],
      cards: [card('t2', 'Dropped', 'Cancelled')],
    })
    expect(columnOf(p).pipeline[0].agents[0].tickets.map(t => t.task)).toEqual(['t1'])
  })
})

describe('the runner’s own plan', () => {
  it('reads as a column with nothing invented: every agent is a real one', () => {
    const p = parsePlan(JSON.stringify(fixturePlan))!
    const c = columnOf(p)
    for (const g of c.pipeline) {
      for (const a of g.agents) {
        expect(a.doing).not.toBe('')
        expect(p.agents.some(x => x.id === a.id)).toBe(true)
      }
    }
    expect(c.facts.some(f => f.kind === 'done')).toBe(true)
  })
})

describe('Needs You', () => {
  it('quotes a task by key and title, else the words, on one line and with nothing that acts in a terminal', () => {
    expect(quote('ov-1', 'Site copy')).toBe('About ov-1 (“Site copy”): ')
    expect(quote(null, 'Allow\nBash?')).toBe('About “Allow Bash?”: ')
    expect(oneLine('a\u001b[201~b\u0003 c​d\u001b]0;x\u0007e')).toBe('ab cde')
  })

  it('lists each initiative’s ask first, then the runner’s items, each with what a click quotes', () => {
    const p = plan({ themes: [theme('Mobile', [], 1, { ask: 'the Play service account' })] })
    const rows = needsRows(p, [
      { id: 'a', since: 5, question: 'Allow Bash?', task: null, terminal: { label: 'Builder 1' } },
      { id: 'b', since: null, question: 'Pick one', task: { key: 'ov-9', title: 'Site copy' }, terminal: null },
    ])
    expect(rows.map(r => [r.title, r.detail])).toEqual([['Mobile', 'the Play service account'], ['Allow Bash?', 'Builder 1'], ['Site copy', 'Pick one']])
    expect(rows[2].quote).toBe('About ov-9 (“Site copy”): ')
  })

  it('says a span the Mac’s way', () => {
    expect([span(47 * MIN), span(2 * 60 * MIN), span(3 * 1440 * MIN), span(-5)]).toEqual(['47 min', '2 h', '3 d', '0 min'])
  })
})
