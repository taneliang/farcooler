// The way back from a dead tunnel (src/client/reconnect.ts, ov-772): the
// notice, the dial made ahead of the click, one touch, and the pane put back.
// Every browser call is played by the test, and time is a number the test sets.

import { describe, expect, it, vi } from 'vitest'

import {
  type Deps, type LinkView, type Linked, GOODBYE_MS, PREDIAL_MAX_AGE_MS, Reconnector, Replacement, paneOutput, recheckOnReturn,
} from '../src/client/reconnect'

class Runner implements Linked {
  freed = false
  answers = true
  /// The wasm client's `onLost`; a closed connection unless a test says.
  onLost: (cause?: string, idleMs?: number) => void
  constructor(lost: (cause: string, idleMs: number) => void, readonly conn: string) {
    this.onLost = (cause = 'socket-close', idleMs = 0) => lost(cause, idleMs)
  }
  signing_out() {
    this.use()
    return 4200
  }
  async check() {
    this.use()
    if (!this.answers) this.onLost()
    return this.answers
  }
  /// A wasm-bindgen object throws on any use after `free`.
  free() {
    this.freed = true
  }
  use(): void {
    if (this.freed) throw new Error('null pointer passed to rust')
  }
}

function rig(options: { dialFails?: () => boolean; connectFails?: () => string | null } = {}) {
  const log: string[] = []
  const views: LinkView[] = []
  const lines: string[] = []
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
    log: line => void lines.push(line),
  }
  const link = new Reconnector(deps)
  return {
    link,
    log,
    lines,
    views,
    runners,
    at: (ms: number) => void (clock = ms),
    last: () => views[views.length - 1],
    settle: () => new Promise<void>(r => setTimeout(r, 0)),
  }
}

describe('a connection that is lost', () => {
  it('says Reconnecting at once, closes the old tunnel and dials ahead of the click', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    first.onLost()
    expect(t.last()).toEqual({ kind: 'reconnecting', tunnel: 'dialing' })
    expect(t.link.current).toBeNull()
    expect(first.freed).toBe(false)
    expect(t.log).toContain('discard tunnel 0')
    expect(t.log).toContain('dial 1')
    expect(t.log.some(l => l.startsWith('touch') && l !== 'touch over tunnel 0')).toBe(false)
    await t.settle()
    expect(t.last()).toEqual({ kind: 'reconnecting', tunnel: 'ready' })
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

describe('what the page asks after a loss', () => {
  it('remembers a lost runner while views may hold it', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    expect(t.link.hasLost(first)).toBe(false)
    first.onLost()
    expect(t.link.hasLost(first)).toBe(true)
    await t.settle()
    await t.link.reconnect()
    expect(t.link.hasLost(t.runners[1])).toBe(false)
    expect(t.link.current).toBe(t.runners[1])
  })

  it('closes a tunnel the login failed on', async () => {
    let refuse = false
    const t = rig({ connectFails: () => (refuse ? 'key_rejected' : null) })
    const first = await t.link.open('tunnel 0')
    first.onLost()
    await t.settle()
    refuse = true
    await t.link.reconnect()
    expect(t.log).toContain('discard tunnel 1')
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

describe('a pane that comes back after a drop', () => {
  it('keeps what it showed until the replay starts, then clears once', () => {
    const replay = new Replacement(true)
    // Nothing has arrived yet: the old screen and scrollback are still shown.
    expect(replay.first()).toBe(true)
    expect(replay.first()).toBe(false)
    expect(replay.first()).toBe(false)
  })

  it('never clears a pane that was opened fresh', () => {
    expect(new Replacement(false).first()).toBe(false)
  })
})

describe('the pane output wiring', () => {
  const run = (resuming: boolean, live = () => true) => {
    const events: string[] = []
    const out = paneOutput(new Replacement(resuming), live, () => events.push('reset'), b => events.push(`write ${b[0]}`))
    return { events, out }
  }

  it('clears a resumed pane before its first chunk only', () => {
    const { events, out } = run(true)
    out(Uint8Array.of(1))
    out(Uint8Array.of(2))
    expect(events).toEqual(['reset', 'write 1', 'write 2'])
  })

  it('never clears a fresh pane', () => {
    const { events, out } = run(false)
    out(Uint8Array.of(1))
    expect(events).toEqual(['write 1'])
  })

  it('writes nothing, and clears nothing, once the page moved on', () => {
    const { events, out } = run(true, () => false)
    out(Uint8Array.of(1))
    expect(events).toEqual([])
  })
})

describe('why a link ended (ov-1513)', () => {
  it.each([
    ['probe-timeout', 14_003.4, 'far cooler: link ended cause=probe-timeout idle_ms=14003'],
    ['silence', 9000, 'far cooler: link ended cause=silence idle_ms=9000'],
    ['socket-close', 12, 'far cooler: link ended cause=socket-close idle_ms=12'],
  ])('logs one line for %s, with the idle time', async (cause, idle, line) => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    first.onLost(cause, idle)
    first.onLost(cause, idle)
    expect(t.lines).toEqual([line])
  })

  it('holds a link it gave up on open long enough to tell the runner, and no longer', async () => {
    vi.useFakeTimers()
    try {
      const t = rig()
      const first = await t.link.open('tunnel 0')
      first.onLost('probe-timeout', 14_000)
      await vi.advanceTimersByTimeAsync(GOODBYE_MS - 1)
      expect(t.log).not.toContain('discard tunnel 0')
      await vi.advanceTimersByTimeAsync(1)
      expect(t.log).toContain('discard tunnel 0')
    } finally {
      vi.useRealTimers()
    }
  })

  it('logs a sign-out with the idle time, tells the runner, and calls what follows no loss', async () => {
    const t = rig()
    const first = await t.link.open('tunnel 0')
    t.link.signingOut()
    t.link.signingOut()
    first.onLost('socket-close', 5)
    expect(t.lines).toEqual(['far cooler: link ended cause=sign-out idle_ms=4200'])
    expect(t.log).not.toContain('dial 1')
    expect(t.log).not.toContain('discard tunnel 0')
  })
})

describe('what makes the page ask whether the link answers', () => {
  function page() {
    const window = new EventTarget()
    const document = Object.assign(new EventTarget(), { visibilityState: 'visible' })
    const recheck = vi.fn()
    recheckOnReturn(window, document, recheck)
    return { window, document, recheck }
  }

  it('asks when the page is restored, comes online, or becomes visible', () => {
    const { window, document, recheck } = page()
    window.dispatchEvent(new Event('pageshow'))
    window.dispatchEvent(new Event('online'))
    document.dispatchEvent(new Event('visibilitychange'))
    expect(recheck).toHaveBeenCalledTimes(3)
  })

  it('does not ask on window focus, or when the page is hidden', () => {
    const { window, document, recheck } = page()
    window.dispatchEvent(new Event('focus'))
    document.visibilityState = 'hidden'
    document.dispatchEvent(new Event('visibilitychange'))
    expect(recheck).not.toHaveBeenCalled()
  })

  // The wiring above is only worth its test if app.ts uses it and adds no
  // other: its own focus listener may check the build, never the link.
  it('is the only thing in app.ts that rechecks the link', async () => {
    const app = (await import('../src/client/app.ts?raw')).default
    expect(app).toContain('recheckOnReturn(window, document, recheckLink)')
    expect(app.match(/recheckLink/g)).toHaveLength(2)
  })
})
