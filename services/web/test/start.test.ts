// The start screen while the page waits (src/client/start.ts, ov-869): the
// step, the seconds once a start is slower than usual, and at ten seconds a
// sentence with a way forward, never "Starting Far Cooler…" alone.

import { describe, expect, it } from 'vitest'

import { SHOW_SECONDS_MS, SLOW_MS, type StartScreen, startScreen, StartWatch, type Ticker } from '../src/client/start'

/// A clock the test moves, with the one interval the watch sets.
function fakeTicker() {
  let now = 1_000_000
  let tick: (() => void) | null = null
  const ticker: Ticker = {
    now: () => now,
    every: run => {
      tick = run
      return 1
    },
    stop: () => {
      tick = null
    },
  }
  return {
    ticker,
    advance(ms: number) {
      for (let left = ms; left > 0; left -= 1000) {
        now += Math.min(1000, left)
        tick?.()
      }
    },
    running: () => tick !== null,
  }
}

describe('startScreen', () => {
  it('names the step and shows no count while a start is quick', () => {
    const quick = startScreen('account', SHOW_SECONDS_MS - 1)
    expect(quick.title).toBe('Checking your account…')
    expect(quick.detail).not.toMatch(/\d s/)
    expect(quick.slow).toBe(false)
  })

  it('counts the seconds once they are worth showing', () => {
    const counting = startScreen('load', 4_200)
    expect(counting.title).toBe('Loading Far Cooler…')
    expect(counting.detail).toContain('4 s')
    expect(counting.slow).toBe(false)
  })

  it('says the start is slow, and why it might be, at ten seconds', () => {
    const slow = startScreen('load', SLOW_MS + 2_000)
    expect(slow.slow).toBe(true)
    expect(slow.detail).toContain('12 s')
    expect(slow.detail).toContain('Reload')
    expect(startScreen('settings', SLOW_MS - 1).slow).toBe(false)
  })
})

describe('StartWatch', () => {
  it('draws at once, moves with the steps, and turns slow at ten seconds', () => {
    const clock = fakeTicker()
    const drawn: StartScreen[] = []
    const watch = new StartWatch(screen => drawn.push(screen), clock.ticker)
    expect(drawn.at(-1)?.title).toBe('Checking your account…')
    clock.advance(2_000)
    watch.at('load')
    expect(drawn.at(-1)?.title).toBe('Loading Far Cooler…')
    clock.advance(8_000)
    // The time counts from the start, not from the step.
    expect(drawn.at(-1)?.slow).toBe(true)
    expect(drawn.at(-1)?.detail).toContain('10 s')
  })

  it('draws nothing once the start is done, and stops its timer', () => {
    const clock = fakeTicker()
    const drawn: StartScreen[] = []
    const watch = new StartWatch(screen => drawn.push(screen), clock.ticker)
    watch.done()
    const count = drawn.length
    clock.advance(SLOW_MS * 2)
    watch.at('settings')
    expect(drawn.length).toBe(count)
    expect(clock.running()).toBe(false)
    expect(watch.finished).toBe(true)
  })
})
