// The way back from a dead tunnel (src/client/reconnect.ts, ov-772): the
// notice, the dial made ahead of the click, one touch, and the pane put back.
// Every browser call is played by the test, and time is a number the test sets.

import { describe, expect, it } from 'vitest'

import { linkNotice } from '../src/client/copy'
import { type Deps, type LinkView, type Linked, PREDIAL_MAX_AGE_MS, Reconnector } from '../src/client/reconnect'

class Runner implements Linked {
  freed = false
  answers = true
  constructor(readonly onLost: () => void, readonly conn: string) {}
  async check() {
    if (!this.answers) this.onLost()
    return this.answers
  }
  free() {
    this.freed = true
  }
}

function rig(options: { dialFails?: () => boolean; connectFails?: () => string | null } = {}) {
  const log: string[] = []
  const views: LinkView[] = []
  let clock = 0
  let dials = 0
  const runners: Runner[] = []
  const deps: Deps<Runner, string> = {
    dial: async () => {
      const n = ++dials
      log.push(`dial ${n}`)
      if (options.dialFails?.()) throw new Error('no_answer')
      return `tunnel ${n}`
    },
    discard: conn => void log.push(`discard ${conn}`),
    connect: async (conn, onLost) => {
      log.push(`touch over ${conn}`)
      const why = options.connectFails?.()
      if (why) throw new Error(why)
      const runner = new Runner(onLost, conn)
      runners.push(runner)
      return runner
    },
    resume: async r => void log.push(`resume on ${r.conn}`),
    show: v => void views.push(v),
    now: () => clock,
  }
  const link = new Reconnector(deps)
  return {
    link,
    log,
    views,
    runners,
    at: (ms: number) => void (clock = ms),
    last: () => views[views.length - 1],
    settle: () => new Promise<void>(r => setTimeout(r, 0)),
  }
}

describe('a connection that is lost', () => {
  it('says Reconnecting at once, frees the old runner and dials ahead of the click', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    first.onLost()
    expect(t.last()).toEqual({ kind: 'reconnecting', tunnel: 'dialing' })
    expect(first.freed).toBe(true)
    expect(t.log).toContain('dial 1')
    expect(t.log.some(l => l.startsWith('touch') && l !== 'touch over tunnel 0')).toBe(false)
    await t.settle()
    expect(t.last()).toEqual({ kind: 'reconnecting', tunnel: 'ready' })
    expect(linkNotice(t.last(), 'studio')?.title).toBe('Reconnecting to studio…')
    expect(linkNotice(t.last(), 'studio')?.action).toBe('Reconnect')
  })

  it('takes one touch on the tunnel it already has, then puts the pane back', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    first.onLost()
    await t.settle()
    t.at(5_000)
    await t.link.reconnect()
    expect(t.log.filter(l => l.startsWith('dial'))).toEqual(['dial 1'])
    expect(t.log.slice(-2)).toEqual(['touch over tunnel 1', 'resume on tunnel 1'])
    expect(t.log.filter(l => l.startsWith('touch'))).toHaveLength(2)
    expect(t.last()).toEqual({ kind: 'up' })
    expect(t.link.current?.conn).toBe('tunnel 1')
  })

  it('dials again at the click when the tunnel dialed ahead has gone stale', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    first.onLost()
    await t.settle()
    t.at(PREDIAL_MAX_AGE_MS + 1)
    await t.link.reconnect()
    expect(t.log).toContain('discard tunnel 1')
    expect(t.log).toContain('touch over tunnel 2')
  })

  it('keeps a tunnel that is exactly as old as the limit', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    first.onLost()
    await t.settle()
    t.at(PREDIAL_MAX_AGE_MS)
    await t.link.reconnect()
    expect(t.log).toContain('touch over tunnel 1')
  })

  it('never hangs: a dial nobody answers ends in failed with the dial word, and Reconnect tries again', async () => {
    let failing = true
    const t = rig({ dialFails: () => failing })
    const first = await t.link.open('tunnel 0')
    first.onLost()
    await t.settle()
    expect(t.last()).toEqual({ kind: 'reconnecting', tunnel: 'failed' })
    await t.link.reconnect()
    expect(t.last()).toEqual({ kind: 'failed', word: 'no_answer' })
    expect(linkNotice(t.last(), 'studio')?.detail).toContain('didn’t answer')
    expect(linkNotice(t.last(), 'studio')?.action).toBe('Reconnect')
    failing = false
    await t.link.reconnect()
    expect(t.last()).toEqual({ kind: 'up' })
  })

  it('shows a refused login as failed with its word, and Reconnect tries again', async () => {
    let refuse: string | null = null
    const t = rig({ connectFails: () => refuse })
    const first = await t.link.open('tunnel 0')
    first.onLost()
    await t.settle()
    refuse = 'key_rejected'
    await t.link.reconnect()
    expect(t.last()).toEqual({ kind: 'failed', word: 'key_rejected' })
    expect(t.link.current).toBeNull()
    refuse = null
    await t.link.reconnect()
    expect(t.last()).toEqual({ kind: 'up' })
  })

  it('ignores news from a connection it already replaced', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    first.onLost()
    await t.settle()
    await t.link.reconnect()
    const views = t.views.length
    first.onLost()
    expect(t.views.length).toBe(views)
    expect(t.last()).toEqual({ kind: 'up' })
    expect(t.runners[1].freed).toBe(false)
  })

  it('does not start a second reconnection while one is under way', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    first.onLost()
    await t.settle()
    await Promise.all([t.link.reconnect(), t.link.reconnect()])
    expect(t.log.filter(l => l.startsWith('touch'))).toHaveLength(2)
  })
})

describe('a tab that comes back', () => {
  it('asks the runner at once, and a silent runner brings the notice up', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    await t.link.recheck()
    expect(t.last()).toEqual({ kind: 'up' })
    first.answers = false
    await t.link.recheck()
    expect(t.last().kind).toBe('reconnecting')
    expect(t.log).toContain('dial 1')
  })

  it('asks nothing while a reconnection is waiting', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    first.onLost()
    let asked = 0
    first.check = async () => ++asked > 0
    await t.link.recheck()
    expect(asked).toBe(0)
  })
})

describe('the words', () => {
  it('names a failure by the dial word the phones use', () => {
    expect(linkNotice({ kind: 'failed', word: 'derp' }, 'studio')?.detail).toContain('rendezvous')
    expect(linkNotice({ kind: 'up' }, 'studio')).toBeNull()
    expect(linkNotice({ kind: 'connecting' }, 'studio')?.action).toBeNull()
  })
})
