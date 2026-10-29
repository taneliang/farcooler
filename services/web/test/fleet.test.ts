// Needs You across every enrolled runner (src/client/fleet.ts, ov-861): the
// runners' lists merged, each row naming its runner, an answer sent to the
// runner the item is on, and a runner that can't be reached a line of its own
// that hides nobody else's items. The runners are played by the test, with the
// lists the Mac and the phones decode (test/fixtures/needs-you.json).

import { describe, expect, it } from 'vitest'

import fixture from '../../../test/fixtures/needs-you.json'
import { Fleet, type RunnerDeps } from '../src/client/fleet'
import { type BoardView, buttonsFor, countOf } from '../src/client/needs'

const list = (name: string) => JSON.stringify((fixture.runners as any[]).find(r => r.runner === name).needs_you)

/// A runner that lists `json` and records what it is asked.
function runner(json: string) {
  const asked: string[] = []
  const deps: RunnerDeps = {
    list: async () => json,
    ask: async (terminal, ask, option) => void asked.push(`ask ${terminal} ${ask} ${option}`),
    decide: async (task, body) => void asked.push(`decide ${task} ${body}`),
  }
  return { deps, asked }
}

function fleet() {
  const views: BoardView[] = []
  const opened: string[] = []
  const set = new Fleet(
    v => views.push(v),
    (r, t) => opened.push(`${r}:${t}`),
    () => true,
  )
  return { set, opened, last: () => views[views.length - 1] }
}
const settle = () => new Promise(r => setTimeout(r, 0))

describe('two runners', () => {
  it('lists both runners’ items, each row naming its runner, in the order they were enrolled', async () => {
    const f = fleet()
    f.set.add('r1', 'studio', runner(list('studio')).deps)
    f.set.add('r2', 'build-box', runner(list('build-box')).deps)
    f.set.up('r1')
    f.set.up('r2')
    await settle()
    const view = f.last()
    expect(view.state).toBe('ready')
    const names = view.items.map(i => i.runnerName)
    expect(new Set(names)).toEqual(new Set(['studio', 'build-box']))
    expect(names.indexOf('build-box')).toBeGreaterThan(names.lastIndexOf('studio'))
    expect(countOf(view.items)).toBeGreaterThan(countOf(view.items.filter(x => x.runner === 'r1')))
    expect(view.items.length).toBe(JSON.parse(list('studio')).items.length + JSON.parse(list('build-box')).items.length)
  })

  it('names no runner when there is only one', async () => {
    const f = fleet()
    f.set.add('r1', 'studio', runner(list('studio')).deps)
    f.set.up('r1')
    await settle()
    expect(f.last().items.every(i => i.runnerName === undefined)).toBe(true)
  })

  it('sends an answer to the runner the item is on, even when both number an item alike', async () => {
    const same = JSON.stringify({ items: [{ id: 'ask:1', kind: 'ask', question: 'Run it?', ask_id: 'a1', terminal: { id: 't1', label: 'x' }, actions: [{ id: 'allow', title: 'Allow' }] }] })
    const a = runner(same)
    const b = runner(same)
    const f = fleet()
    f.set.add('r1', 'studio', a.deps)
    f.set.add('r2', 'build-box', b.deps)
    f.set.up('r1')
    f.set.up('r2')
    await settle()
    const second = f.last().items.find(i => i.runner === 'r2')!
    const [button] = buttonsFor(second)
    await f.set.press(second, button.press)
    expect(a.asked).toEqual([])
    expect(b.asked).toEqual(['ask t1 a1 allow'])
    expect(f.last().busy.size).toBe(0)
  })

  it('marks only the item answered as busy, not the other runner’s item with the same id', async () => {
    const same = JSON.stringify({ items: [{ id: 'ask:1', kind: 'ask', question: 'Run it?', ask_id: 'a1', terminal: { id: 't1', label: 'x' }, actions: [{ id: 'allow', title: 'Allow' }] }] })
    let release = () => {}
    const slow: RunnerDeps = { ...runner(same).deps, ask: () => new Promise(done => (release = done)) }
    const f = fleet()
    f.set.add('r1', 'studio', runner(same).deps)
    f.set.add('r2', 'build-box', slow)
    f.set.up('r1')
    f.set.up('r2')
    await settle()
    const second = f.last().items.find(i => i.runner === 'r2')!
    const answering = f.set.press(second, buttonsFor(second)[0].press)
    await settle()
    const first = f.last().items.find(i => i.runner === 'r1')!
    expect(f.last().busy.has(second.id)).toBe(true)
    expect(f.last().busy.has(first.id)).toBe(false)
    release()
    await answering
  })

  it('shows one runner unreachable without hiding the other’s items', async () => {
    const f = fleet()
    f.set.add('r1', 'studio', runner(list('studio')).deps)
    f.set.add('r2', 'build-box', runner(list('build-box')).deps)
    f.set.up('r1')
    f.set.unreachable('r2', 'build-box didn’t answer.', 'Connect')
    await settle()
    const view = f.last()
    expect(view.state).toBe('ready')
    expect(view.items.length).toBe(JSON.parse(list('studio')).items.length)
    expect(view.items.every(i => i.runner === 'r1')).toBe(true)
    expect(view.runners).toEqual([{ runner: 'r2', text: 'build-box didn’t answer.', retry: 'Connect' }])
  })

  it('keeps a runner’s last list under its note once it stops answering, and reads it again when it is back', async () => {
    const f = fleet()
    f.set.add('r1', 'studio', runner(list('studio')).deps)
    f.set.add('r2', 'build-box', runner(list('build-box')).deps)
    f.set.up('r1')
    f.set.up('r2')
    await settle()
    const all = f.last().items.length
    f.set.unreachable('r2', 'gone', 'Reconnect')
    expect(f.last().items.length).toBe(all)
    expect(f.last().runners).toHaveLength(1)
    f.set.up('r2')
    await settle()
    expect(f.last().runners).toEqual([])
  })

  it('says a runner is being connected to, and lists a runner it can never reach', async () => {
    const f = fleet()
    f.set.add('r1', 'studio', runner(list('studio')).deps)
    f.set.addUnreachable('r2', 'nas', 'nas is only reachable at an address.')
    f.set.add('r3', 'build-box', runner(list('build-box')).deps)
    f.set.up('r1')
    f.set.connecting('r3')
    await settle()
    expect(f.last().runners?.map(n => [n.runner, n.text, n.retry])).toEqual([
      ['r2', 'nas is only reachable at an address.', undefined],
      ['r3', 'Connecting to build-box…', undefined],
    ])
    expect(f.last().state).toBe('ready')
  })

  it('has nothing to list while every runner is unreachable, and says why for each', async () => {
    const f = fleet()
    f.set.add('r1', 'studio', runner(list('studio')).deps)
    f.set.addUnreachable('r2', 'nas', 'nas: address only')
    f.set.unreachable('r1', 'studio: no answer', 'Connect')
    await settle()
    expect(f.last().state).toBe('unreadable')
    expect(f.last().items).toEqual([])
    expect(f.last().runners?.map(n => n.runner)).toEqual(['r1', 'r2'])
  })

  it('reads a failed list as that runner’s note, not the others’', async () => {
    const f = fleet()
    f.set.add('r1', 'studio', runner(list('studio')).deps)
    f.set.add('r2', 'build-box', { ...runner('{}').deps, list: async () => Promise.reject(new Error('no_answer')) })
    f.set.up('r1')
    f.set.up('r2')
    await settle()
    expect(f.last().state).toBe('ready')
    expect(f.last().runners?.map(n => n.runner)).toEqual(['r2'])
    expect(f.last().runners?.[0].text).toContain('build-box')
  })

  it('goes to the terminal on the runner the item is on', async () => {
    const f = fleet()
    f.set.add('r1', 'studio', runner(list('studio')).deps)
    f.set.up('r1')
    await settle()
    const item = f.last().items.find(i => i.terminal)!
    await f.set.press(item, { kind: 'open', terminal: item.terminal!.id })
    expect(f.opened).toEqual([`r1:${item.terminal!.id}`])
  })
})
