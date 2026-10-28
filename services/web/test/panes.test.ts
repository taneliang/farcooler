// Many panes on one connection (src/client/panes.ts, ov-774): the channel
// budget, which pane lets go first, and the way back to a pane let go of.
// The terminals are played by the test; a channel is a number it counts.

import { describe, expect, it } from 'vitest'

import { MAX_LIVE_PANES, type Pane, PaneSet } from '../src/client/panes'

/// What the connection's sshd sees: channels open now, and the most there were.
class Sshd {
  open = 0
  peak = 0
  /// Refuses a channel past this many, as `MaxSessions` does (the control
  /// channel is one of them, so 10 here is 9 panes).
  constructor(readonly sessions = 10) {}
  take(): boolean {
    if (this.open + 1 >= this.sessions) return false
    this.open++
    this.peak = Math.max(this.peak, this.open + 1)
    return true
  }
  give(): void {
    this.open--
  }
}

class Screen implements Pane<Sshd> {
  active = false
  used = false
  visible = false
  attaches: boolean[] = []
  ends: (() => void) | null = null
  constructor(readonly id: string) {}
  show(on: boolean) {
    this.visible = on
  }
  async attach(runner: Sshd, _terminal: string, ended: () => void, resuming: boolean) {
    this.detach()
    this.attaches.push(resuming)
    if (!runner.take()) throw new Error('exec_failed')
    this.active = true
    this.used = true
    this.ends = ended
    this.runner = runner
  }
  runner: Sshd | null = null
  detach() {
    if (this.active) this.runner?.give()
    this.active = false
  }
  dispose() {
    this.detach()
  }
}

function rig(max?: number, sessions = 10) {
  const sshd = new Sshd(sessions)
  const screens = new Map<string, Screen>()
  const set = new PaneSet<Sshd>(id => {
    const screen = new Screen(id)
    screens.set(id, screen)
    return screen
  }, max)
  const ids = Array.from({ length: 12 }, (_, i) => `t${i + 1}`)
  const ended: string[] = []
  const open = (id: string) => set.open(id, sshd, () => ended.push(id))
  return { sshd, screens, set, ids, open, ended }
}

describe('the channel budget', () => {
  it('opening twelve panes one after another never holds more than ten channels', async () => {
    const { sshd, set, ids, open } = rig()
    for (const id of ids) await open(id)
    expect(sshd.peak).toBeLessThanOrEqual(10)
    expect(set.live).toHaveLength(MAX_LIVE_PANES)
    expect(set.shown).toBe('t12')
  })

  it('lets the least recently shown hidden pane go first', async () => {
    const { screens, set, ids, open } = rig()
    for (const id of ids) await open(id)
    // t1..t5 were let go as t8..t12 came; t6..t12 are attached.
    expect(set.live).toEqual(['t6', 't7', 't8', 't9', 't10', 't11', 't12'])
    expect(screens.get('t1')?.active).toBe(false)
    await open('t6')
    await open('t13')
    // t6 was shown again, so t7 was the least recent, and went.
    expect(screens.get('t6')?.active).toBe(true)
    expect(screens.get('t7')?.active).toBe(false)
  })

  it('keeps a hidden pane attached while there is room, so it goes on scrolling', async () => {
    const { screens, ids, open } = rig()
    for (const id of ids.slice(0, 3)) await open(id)
    expect(ids.slice(0, 3).map(id => screens.get(id)?.active)).toEqual([true, true, true])
    expect(ids.slice(0, 3).map(id => screens.get(id)?.visible)).toEqual([false, false, true])
  })

  it('returning to a pane let go of attaches it again, keeping its screen', async () => {
    const { screens, sshd, ids, open } = rig()
    for (const id of ids) await open(id)
    const first = screens.get('t1') as Screen
    expect(first.active).toBe(false)
    await open('t1')
    expect(first.active).toBe(true)
    expect(first.attaches).toEqual([false, true])
    expect(sshd.peak).toBeLessThanOrEqual(10)
  })

  it('opening the pane already in front changes nothing', async () => {
    const { screens, open } = rig()
    await open('t1')
    await open('t1')
    expect(screens.get('t1')?.attaches).toEqual([false])
  })

  it('tries once more, after letting one go, when the runner refuses a channel', async () => {
    // A runner allowing fewer channels than the budget assumes.
    const { sshd, screens, set, ids, open } = rig(8, 5)
    for (const id of ids.slice(0, 4)) await open(id)
    expect(set.live).toHaveLength(4)
    await open('t5')
    expect(screens.get('t5')?.active).toBe(true)
    expect(screens.get('t1')?.active).toBe(false)
    expect(sshd.open).toBe(4)
  })

  it('says so when no channel can be had', async () => {
    const { open } = rig(8, 1)
    await expect(open('t1')).rejects.toThrow('exec_failed')
  })
})

describe('a new connection', () => {
  it('attaches every pane that was attached, the one in front first, and not those let go', async () => {
    const { screens, set, ids, open, ended } = rig()
    for (const id of ids) await open(id)
    await open('t9')
    const dropped = new Sshd()
    for (const screen of screens.values()) screen.ends?.()
    const fresh = new Sshd()
    const order: string[] = []
    for (const screen of screens.values()) {
      const attach = screen.attach.bind(screen)
      screen.attach = async (...args) => {
        order.push(screen.id)
        return attach(...args)
      }
    }
    await set.resume(fresh, () => ended.push('resume'))
    expect(order[0]).toBe('t9')
    expect(new Set(order)).toEqual(new Set(['t6', 't7', 't8', 't9', 't10', 't11', 't12']))
    expect(fresh.peak).toBeLessThanOrEqual(10)
    expect(dropped.open).toBe(0)
    for (const id of order) expect(screens.get(id)?.attaches.at(-1)).toBe(true)
  })

  it('does not count the lost connection’s channels against the new one', async () => {
    const { screens, set, ids, open } = rig()
    for (const id of ids.slice(0, MAX_LIVE_PANES)) await open(id)
    // The old connection died without its panes being told yet.
    const fresh = new Sshd()
    await set.resume(fresh, () => {})
    expect(set.live).toHaveLength(MAX_LIVE_PANES)
    expect([...screens.values()].filter(s => s.active)).toHaveLength(MAX_LIVE_PANES)
  })

  it('reports the front pane’s failure and carries on with the rest', async () => {
    const { screens, set, ids, open } = rig()
    for (const id of ids.slice(0, 3)) await open(id)
    const fresh = new Sshd()
    const hidden = screens.get('t1') as Screen
    hidden.attach = async () => {
      throw new Error('refused')
    }
    await set.resume(fresh, () => {})
    expect(screens.get('t3')?.active).toBe(true)
    const front = screens.get('t3') as Screen
    front.attach = async () => {
      throw new Error('refused')
    }
    await expect(set.resume(new Sshd(), () => {})).rejects.toThrow('refused')
  })
})

describe('a stream that ends', () => {
  it('is told to the page only for the pane in front', async () => {
    const { screens, ended, open } = rig()
    await open('t1')
    await open('t2')
    screens.get('t1')?.ends?.()
    expect(ended).toEqual([])
    screens.get('t2')?.ends?.()
    expect(ended).toEqual(['t2'])
  })
})
