// The plan view model's request and answer (src/client/plan-present.ts,
// ov-1016). The presenter is injected: wasm can't load under vitest's worker
// pool, so a stub answers with the golden (test/fixtures/plan-view-apple.json,
// which the Rust test pins to crates/client's own answer).

import { describe, expect, it } from 'vitest'

import golden from '../../../test/fixtures/plan-view-apple.json'
import { presentPlan, type PlanPresenter } from '../src/client/plan-present'

const stub = (seen: any[] = []): PlanPresenter => request => {
  seen.push(JSON.parse(request))
  return JSON.stringify(golden.answer)
}

describe('asking for the plan view', () => {
  it('sends the plan as the runner sent it, in the web’s style, with the board’s statuses, the Needs You count and the workflow', () => {
    const seen: any[] = []
    presentPlan(stub(seen), golden.request.plan, { needsYou: 3 })
    expect(seen).toHaveLength(1)
    expect(seen[0].schema).toBe(1)
    expect(seen[0].plan).toEqual(golden.request.plan)
    // The board's titles go with it, so a ticket's heading falls back to them before its key.
    expect(seen[0].context.titles['t-ov-301']).toBe(golden.request.plan.cards.find(c => c.task === 't-ov-301')!.title)
    expect(seen[0].context.style).toBe('apple')
    expect(seen[0].context.needs_you).toBe(3)
    expect(seen[0].context.agents_supported).toBe(true)
    expect(seen[0].context.statuses['t-ov-300']).toBe('Needs Decision')
  })

  it('says agents aren’t listed by a runner that sends none', () => {
    const seen: any[] = []
    const { agents: _, ...older } = golden.request.plan
    presentPlan(stub(seen), older, { needsYou: 0 })
    expect(seen[0].context.agents_supported).toBe(false)
  })

  it('returns the view the presenter answered: the column’s groups in its order', () => {
    const view = presentPlan(stub(), golden.request.plan, { needsYou: 2 })!
    expect(view.column.pipeline.map(g => g.title)).toEqual(['Stuck', 'Building', 'Reviewing', 'Landing'])
    expect(view.column.facts.map(f => f.text)).toEqual(['2 need you', '2 stuck'])
  })

  it('returns nothing for an invalid answer or a presenter that throws, never the runner’s plan as it was', () => {
    expect(presentPlan(() => '{"schema":1,"view":null,"error":"invalid"}', golden.request.plan, { needsYou: 0 })).toBeNull()
    expect(presentPlan(() => 'not json', golden.request.plan, { needsYou: 0 })).toBeNull()
    expect(presentPlan(() => { throw new Error('wasm') }, golden.request.plan, { needsYou: 0 })).toBeNull()
  })
})
