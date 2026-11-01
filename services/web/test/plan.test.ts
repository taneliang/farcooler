// The plan in the browser (src/client/plan.ts, ov-775): the runner's plan as
// every device reads it, the sections the Mac sorts it into, the edits a
// person can make, and what a refused one says. The runner is played by the
// test; the plan is the fixture the Mac and both phones decode
// (test/fixtures/plan.json at the repo root, also `farcooler plan --json`).

import { describe, expect, it } from 'vitest'

import plan from '../../../test/fixtures/plan.json'
import { planRefusal } from '../src/client/copy'
import {
  openRulings,
  PLAN_POLL_MS,
  PlanBoard,
  type PlanBoardView,
  type PlanDeps,
  parsePlan,
  pastRulings,
} from '../src/client/plan'

const WS = '00000000-0000-0000-0000-00000000aaaa'
const THEME = plan.themes[0].id
const RULING = plan.rulings.find(r => r.state === 'standing')!.id
const fixture = () => parsePlan(JSON.stringify(plan))!

describe('the plan', () => {
  // ov-775 acceptance 1: the same plan the Mac decodes, for the same runner.
  it('keeps the runner’s lanes, trains, themes and rulings, in its order, as the fixture the Mac decodes holds them', () => {
    const shown = fixture()
    expect(shown.lanes.map(l => l.id)).toEqual(plan.lanes.map(l => l.id))
    expect(shown.lanes.map(l => l.state)).toEqual(plan.lanes.map(l => l.state))
    expect(shown.trains.map(t => t.heading)).toEqual(plan.trains.map(t => t.title))
    expect(shown.themes.map(t => [t.name, t.next, t.ask])).toEqual(plan.themes.map(t => [t.name, t.next, t.owner_ask]))
    expect(shown.rulings.map(r => [r.short, r.decision, r.state])).toEqual(plan.rulings.map(r => [r.short, r.decision, r.state]))
    expect(shown.order).toEqual(plan.order)
  })

  it('reads a train’s phase from its state when the runner sent none', () => {
    expect(fixture().trains[0].phase).toBe('ci_failed')
    const wire: any = JSON.parse(JSON.stringify(plan))
    delete wire.trains[0].phase
    wire.trains[0].state = 'gating'
    expect(parsePlan(JSON.stringify(wire))!.trains[0].phase).toBe('gating_locally')
    wire.trains[0].state = 'pushed'
    expect(parsePlan(JSON.stringify(wire))!.trains[0].phase).toBe('waiting_for_ci')
  })

  it('shows open rulings newest first and past ones most recently settled first', () => {
    expect(openRulings(fixture()).map(r => r.short)).toEqual(['R-2'])
    expect(pastRulings(fixture()).map(r => r.short)).toEqual(['R-1'])
  })

  it('reads nothing from what is not a plan, and leaves out a row with no id', () => {
    expect(parsePlan('not json')).toBeNull()
    expect(parsePlan('{}')).toBeNull()
    const wire: any = JSON.parse(JSON.stringify(plan))
    wire.lanes.push({ name: 'no id' }, null)
    wire.rulings.push({ id: 'r', decision: 5 })
    const shown = parsePlan(JSON.stringify(wire))!
    expect(shown.lanes).toHaveLength(plan.lanes.length)
    expect(shown.rulings).toHaveLength(plan.rulings.length)
  })
})

// A runner played by hand: it holds a plan, takes edits and says no.
function runner() {
  const state = {
    plan: JSON.parse(JSON.stringify(plan)),
    nexts: [] as [string, string][],
    kept: [] as string[],
    keptAll: 0,
    refuse: null as null | { code: string; what: string },
    reads: 0,
    views: [] as PlanBoardView[],
    fail: null as null | string,
  }
  const reject = () => Promise.reject(Object.assign(new Error('refused'), state.refuse))
  const deps: PlanDeps = {
    runner: 'studio',
    workspaces: async () => JSON.stringify([{ id: WS, name: 'Main', is_main: true }]),
    plan: async ws => {
      expect(ws).toBe(WS)
      state.reads++
      if (state.fail) throw new Error(state.fail)
      return JSON.stringify(state.plan)
    },
    setNext: async (theme, next) => {
      if (state.refuse) return reject()
      state.nexts.push([theme, next])
      state.plan.themes.find((t: any) => t.id === theme).next = next
    },
    keep: async id => {
      if (state.refuse) return reject()
      state.kept.push(id)
      state.plan.rulings.find((r: any) => r.id === id).state = 'confirmed'
    },
    keepAll: async () => {
      if (state.refuse) return reject()
      state.keptAll++
      return 1
    },
    show: v => void state.views.push(v),
  }
  return { state, board: new PlanBoard(deps), last: () => state.views[state.views.length - 1] }
}

describe('a plan card’s cards', () => {
  it('keeps each card’s task beside its key, to open its detail', () => {
    const wire: any = JSON.parse(JSON.stringify(plan))
    wire.lanes[0].cards = [{ task: 'task-1', key: 'ov-1' }, { key: 'no-task' }]
    wire.trains[0].card = { task: 'task-9', key: 'ov-9' }
    const shown = parsePlan(JSON.stringify(wire))!
    expect(shown.lanes[0].cards).toEqual(['ov-1', 'no-task'])
    expect(shown.lanes[0].cardRefs).toEqual([{ task: 'task-1', key: 'ov-1' }])
    expect(shown.trains[0].cardRef).toEqual({ task: 'task-9', key: 'ov-9' })
    wire.trains[0].card = null
    expect(parsePlan(JSON.stringify(wire))!.trains[0].cardRef).toBeNull()
  })
})

describe('the page’s workspace', () => {
  const OTHER = '01990000-0000-7000-8000-0000000000b2'

  it('reads the workspace the page asked for first, not the main one', async () => {
    const asked: string[] = []
    const views: PlanBoardView[] = []
    const board = new PlanBoard({
      runner: 'studio',
      workspace: OTHER,
      workspaces: async () => JSON.stringify([{ id: WS, name: 'Main', is_main: true }, { id: OTHER, name: 'Billing', is_main: false }]),
      plan: async ws => {
        asked.push(ws)
        return JSON.stringify(plan)
      },
      setNext: async () => {},
      keep: async () => {},
      keepAll: async () => 0,
      show: v => void views.push(v),
    })
    await board.refresh()
    expect(asked).toEqual([OTHER])
    expect(views[views.length - 1].workspace).toBe(OTHER)
    board.choose(WS)
    await board.refresh()
    expect(asked).toEqual([OTHER, WS])
  })
})

describe('reading and editing', () => {
  it('reads the first board’s plan', async () => {
    const { board, last } = runner()
    await board.refresh()
    expect(last().state).toBe('ready')
    expect(last().plan?.themes[0].next).toBe('Frosted window plane')
  })

  // ov-775 acceptance 2, browser to Mac: the edit is the runner's own RPC with
  // the text written, and the screen then shows what the runner holds.
  it('sends an edited Next line, trimmed, and shows the plan as the runner now holds it', async () => {
    const { board, state, last } = runner()
    await board.refresh()
    board.edit(THEME)
    board.type('  Glass sidebar  ')
    await board.save()
    expect(state.nexts).toEqual([[THEME, 'Glass sidebar']])
    expect(last().plan?.themes[0].next).toBe('Glass sidebar')
    expect(last().editing).toBeNull()
  })

  it('sends nothing for a Next line left as it was', async () => {
    const { board, state } = runner()
    await board.refresh()
    board.edit(THEME)
    await board.save()
    expect(state.nexts).toEqual([])
  })

  // ov-775 acceptance 2, Mac to browser: a poll shows a change made elsewhere,
  // and polls come inside the two seconds.
  it('shows a change made on another device on the next read, which comes within two seconds', async () => {
    expect(PLAN_POLL_MS).toBeLessThanOrEqual(1500)
    const { board, state, last } = runner()
    await board.refresh()
    state.plan.themes[0].next = 'Set on the Mac'
    await board.refresh()
    expect(last().plan?.themes[0].next).toBe('Set on the Mac')
  })

  it('does not draw over a Next line being typed when a read lands', async () => {
    const { board, state, last } = runner()
    await board.refresh()
    board.edit(THEME)
    board.type('half-writ')
    const drawn = state.views.length
    state.plan.themes[0].next = 'Set on the Mac'
    await board.refresh()
    expect(state.views.length).toBe(drawn)
    // What was typed is still what would be sent.
    await board.save()
    expect(state.nexts).toEqual([[THEME, 'half-writ']])
    expect(last().plan?.themes[0].next).toBe('half-writ')
  })

  it('keeps a ruling, and every open one', async () => {
    const { board, state, last } = runner()
    await board.refresh()
    await board.keep(RULING)
    expect(state.kept).toEqual([RULING])
    expect(openRulings(last().plan!)).toEqual([])
    await board.keepAll()
    expect(state.keptAll).toBe(1)
  })

  // ov-775 acceptance 3: a change the runner won't take from this device comes
  // back as its refusal, in plain words, and the line stays open to be fixed.
  it('shows the runner’s refusal of an owner’s call in words, and keeps what was typed', async () => {
    const { board, state, last } = runner()
    await board.refresh()
    state.refuse = { code: 'not-yours', what: 'not_yours' }
    board.edit(THEME)
    board.type('Mine')
    await board.save()
    expect(state.nexts).toEqual([])
    expect(last().notices.get(THEME)).toBe('studio says that one is the owner’s call. Ask the owner, or ask the orchestrator to change it.')
    expect(last().notices.get(THEME)).not.toMatch(/not.yours/i)
    expect(last().editing).toEqual({ theme: THEME, draft: 'Mine' })
    await board.keep(RULING)
    expect(last().notices.get(RULING)).toContain('owner’s call')
    await board.keepAll()
    expect(last().notices.get('all')).toContain('owner’s call')
  })

  it('words each other refusal, and never the runner’s code', () => {
    const say = (code: string | undefined, word = 'refused') => planRefusal({ word, code }, 'studio', 'The Next line')
    expect(say('scope-denied')).toContain('can only look at studio')
    expect(say('capability-unsupported')).toContain('too old')
    expect(say('not-found')).toContain('isn’t in the plan anymore')
    expect(say('resource-conflict')).toContain('Someone changed this first')
    expect(say('internal')).toBe('studio didn’t take that change. Try again.')
    expect(say(undefined, 'closed')).toContain('The Next line may not have changed')
    for (const code of ['scope-denied', 'not-yours', 'resource-conflict']) expect(say(code)).not.toContain(code)
  })

  it('says a runner too old to show the plan, and a read that failed after a good one', async () => {
    const { board, state, last } = runner()
    state.fail = 'runner_too_old'
    await board.refresh()
    expect(last().state).toBe('old')
    expect(last().banner).toBe('Update Far Cooler on studio to see the plan here.')

    const second = runner()
    await second.board.refresh()
    second.state.fail = 'closed'
    await second.board.refresh()
    expect(second.last().state).toBe('ready')
    expect(second.last().plan).not.toBeNull()
    expect(second.last().banner).toContain('may be behind')
  })
})
