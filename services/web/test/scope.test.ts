// One workspace at a time (src/client/scope.ts, ov-871): the terminals, the
// Needs You items and the counts of the workspace in front, from lists the
// runner wrote (test/fixtures/needs-you.json is the one every client decodes).

import { describe, expect, it } from 'vitest'

import fixture from '../../../test/fixtures/needs-you.json'
import { Fleet, type RunnerDeps } from '../src/client/fleet'
import { type BoardView, countOf, parseList } from '../src/client/needs'
import { firstToOpen, noTerminals, optionLabel, ownerOf, parseTerminals, pickSpace, scopeBoard, terminalsIn, waitingIn } from '../src/client/scope'

const BILLING = '01000000-0000-7000-8000-000000000001'
const MAIN = '01000000-0000-7000-8000-000000000014'
const spaces = [
  { id: MAIN, name: 'Main', isMain: true },
  { id: BILLING, name: 'Billing', isMain: false },
]

const terminals = parseTerminals(
  JSON.stringify([
    { id: 't1', title: 'shell 1', state: 'running', agent: false, workspace: MAIN },
    { id: 't2', title: 'unclaimed', state: 'exited', agent: false },
    { id: 't3', title: 'invoice agent', state: 'running', agent: true, workspace: BILLING },
    { id: 't4', title: 'gone', state: 'running', agent: false, workspace: 'a-deleted-workspace' },
    { id: 't5', title: 'invoice shell', state: 'exited', agent: false, workspace: BILLING },
  ]),
)

const settle = () => new Promise(r => setTimeout(r, 0))
const list = (name: string) => JSON.stringify((fixture.runners as any[]).find(r => r.runner === name).needs_you)
const deps = (json: string): RunnerDeps => ({ list: async () => json, ask: async () => {}, decide: async () => {} })

async function merged(): Promise<BoardView> {
  let last!: BoardView
  const f = new Fleet(
    v => (last = v),
    () => {},
    () => true,
  )
  f.add('r1', 'studio', deps(list('studio')))
  f.add('r2', 'build-box', deps(list('build-box')))
  f.up('r1')
  f.up('r2')
  await settle()
  return last
}

describe('which workspace owns what', () => {
  it('reads a terminal’s workspace, and none as none', () => {
    expect(terminals.map(t => t.workspace)).toEqual([MAIN, null, BILLING, 'a-deleted-workspace', BILLING])
    expect(parseTerminals('nonsense')).toEqual([])
  })

  it('gives a terminal with no workspace, or a gone one, to the main workspace', () => {
    expect(ownerOf(null, spaces)).toBe(MAIN)
    expect(ownerOf('a-deleted-workspace', spaces)).toBe(MAIN)
    expect(ownerOf(BILLING, spaces)).toBe(BILLING)
    expect(ownerOf(null, [])).toBeNull()
  })

  it('lists only the workspace in front’s terminals, in the runner’s order', () => {
    expect(terminalsIn(terminals, spaces, MAIN).map(t => t.id)).toEqual(['t1', 't2', 't4'])
    expect(terminalsIn(terminals, spaces, BILLING).map(t => t.id)).toEqual(['t3', 't5'])
  })

  it('lists every terminal when the runner has no workspaces', () => {
    expect(terminalsIn(terminals, [], null).length).toBe(5)
  })

  it('picks the saved workspace, else the main one, else the first', () => {
    expect(pickSpace(spaces, BILLING)).toBe(BILLING)
    expect(pickSpace(spaces, 'gone')).toBe(MAIN)
    expect(pickSpace(spaces, null)).toBe(MAIN)
    expect(pickSpace([{ id: 'x', name: 'X', isMain: false }], null)).toBe('x')
    expect(pickSpace([], 'x')).toBeNull()
  })

  it('opens the first running terminal, else the first', () => {
    expect(firstToOpen(terminalsIn(terminals, spaces, BILLING))?.id).toBe('t3')
    expect(firstToOpen([{ ...terminals[1] }])?.id).toBe('t2')
    expect(firstToOpen([])).toBeUndefined()
  })
})

describe('Needs You for one workspace', () => {
  it('keeps the runner in front’s items of the workspace in front, and lists the other runners’ apart', async () => {
    const view = await merged()
    const inBilling = scopeBoard(view, spaces, BILLING, 'r1')
    const mine = inBilling.items.filter(i => !i.elsewhere)
    expect(mine.length).toBeGreaterThan(0)
    expect(mine.every(i => i.runner === 'r1' && i.workspaceId === BILLING)).toBe(true)
    expect(inBilling.items.filter(i => i.elsewhere).every(i => i.runner === 'r2')).toBe(true)
    expect(inBilling.items.filter(i => i.elsewhere).length).toBe(JSON.parse(list('build-box')).items.length)

    const inMain = scopeBoard(view, spaces, MAIN, 'r1')
    // The studio’s item with no workspace is the main workspace’s.
    expect(inMain.items.filter(i => !i.elsewhere).map(i => i.kind)).toEqual(['pr_review'])
    expect(countOf(inMain.items.filter(i => !i.elsewhere))).toBe(1)
  })

  it('carries a busy flag and a refusal only for the items it keeps', async () => {
    const view = await merged()
    const billing = scopeBoard(view, spaces, BILLING, 'r1').items.find(i => !i.elsewhere)!
    const main = scopeBoard(view, spaces, MAIN, 'r1').items.find(i => !i.elsewhere)!
    const busy: BoardView = { ...view, busy: new Set([billing.id, main.id]), notices: new Map([[billing.id, 'no'], [main.id, 'nope']]) }
    expect([...scopeBoard(busy, spaces, BILLING, 'r1').busy]).toEqual([billing.id])
    expect([...scopeBoard(busy, spaces, MAIN, 'r1').notices.keys()]).toEqual([main.id])
  })

  it('cuts nothing when the runner has no workspaces', async () => {
    const view = await merged()
    expect(scopeBoard(view, [], null, 'r1')).toBe(view)
  })

  it('counts what waits in each workspace of the runner in front, and not the others’ runners', async () => {
    const view = await merged()
    const counts = waitingIn(view, spaces, 'r1')
    expect(counts.get(BILLING)).toBe(countOf(parseList(list('studio')).filter(i => i.workspaceId === BILLING)))
    expect(counts.get(MAIN)).toBe(1)
    expect(optionLabel(spaces[1], counts.get(BILLING)!)).toBe(`Billing (${counts.get(BILLING)})`)
    expect(optionLabel(spaces[0], 0)).toBe('Main')
  })
})

describe('words', () => {
  it('names the workspace that has no terminals, when there are several', () => {
    expect(noTerminals(spaces[1], 'studio', true)).toBe('Billing has no terminals yet. Open one in Far Cooler on your Mac or phone, then reload.')
    expect(noTerminals(undefined, 'studio', false)).toBe('studio has no terminals yet. Open one in Far Cooler on your Mac or phone, then reload.')
  })
})
