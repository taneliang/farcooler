// The Use Passkey screen's rule (src/client/passkey-prompt.ts, ov-767 fix
// round 1): Use Passkey signs once, Cancel and the window each end it as
// declined, and nothing after the end asks the passkey for a touch.

import { describe, expect, it } from 'vitest'

import { askToUsePasskey, type Timers, USE_PASSKEY_WINDOW_MS } from '../src/client/passkey-prompt'

/// Timers the test fires by hand.
function manualTimers() {
  const pending = new Map<number, { run: () => void; ms: number }>()
  let next = 0
  const timers: Timers = {
    set: (run, ms) => {
      pending.set(++next, { run, ms })
      return next
    },
    clear: handle => void pending.delete(handle as number),
  }
  return {
    timers,
    pending,
    fire: () => {
      for (const [id, t] of [...pending]) {
        pending.delete(id)
        t.run()
      }
    },
  }
}

/// The screen, as the page shows it: its two buttons, held for the test.
function screen() {
  const buttons: { use?: () => void; cancel?: () => void } = {}
  return { buttons, show: (use: () => void, cancel: () => void) => Object.assign(buttons, { use, cancel }) }
}

function counting() {
  let touches = 0
  return { sign: async () => `signed ${++touches}`, touches: () => touches }
}

describe('Use Passkey', () => {
  it('waits well inside sshd’s 120 s LoginGraceTime', () => {
    const { timers, pending } = manualTimers()
    const { show } = screen()
    void askToUsePasskey(show, async () => 'x', undefined, timers).catch(() => {})
    expect([...pending.values()][0].ms).toBe(USE_PASSKEY_WINDOW_MS)
    expect(USE_PASSKEY_WINDOW_MS).toBeLessThanOrEqual(60_000)
  })

  it('signs once from the click, and a second click asks nothing', async () => {
    const { timers, pending } = manualTimers()
    const { buttons, show } = screen()
    const passkey = counting()
    const answer = askToUsePasskey(show, passkey.sign, 1000, timers)
    buttons.use!()
    buttons.use!()
    buttons.cancel!()
    await expect(answer).resolves.toBe('signed 1')
    expect(passkey.touches()).toBe(1)
    expect(pending.size).toBe(0)
  })

  it('ends as declined on Cancel, and a late click spends no touch', async () => {
    const { timers, pending } = manualTimers()
    const { buttons, show } = screen()
    const passkey = counting()
    const answer = askToUsePasskey(show, passkey.sign, 1000, timers)
    buttons.cancel!()
    buttons.use!()
    await expect(answer).rejects.toThrow('declined')
    expect(passkey.touches()).toBe(0)
    expect(pending.size).toBe(0)
  })

  it('ends as declined when the window closes, and a late click spends no touch', async () => {
    const { timers, fire } = manualTimers()
    const { buttons, show } = screen()
    const passkey = counting()
    const answer = askToUsePasskey(show, passkey.sign, 1000, timers)
    fire()
    buttons.use!()
    await expect(answer).rejects.toThrow('declined')
    expect(passkey.touches()).toBe(0)
  })
})
