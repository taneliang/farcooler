import { describe, expect, it } from 'vitest'

import { countsOf, type Machine } from '../src/index'

// One pull request counts once in the push total (ov-590). Two runners signed
// in to one gh login are each asked to review the same pull requests, and the
// relay, which sees no repository, title, URL or login, can only tell how many.

const now = 1_700_000_000_000

function runner(id: string, needsYou: number | null, prReviews?: number | null): Machine {
  return {
    id,
    label: id,
    name: id,
    install_id: id,
    needs_you: needsYou,
    needs_you_at: needsYou === null ? null : now,
    pr_reviews: prReviews,
    last_seen_at: now,
    beat_every: 300,
    expires_at: null,
  }
}

describe('the needs-you total with pull-request review requests', () => {
  it('counts a pull request two runners were both asked about once', () => {
    // Each runner: one ask and the same two pull requests. Summed this was 6.
    const counts = countsOf([runner('a', 3, 2), runner('b', 3, 2)], now)
    expect(counts?.total).toBe(4)
  })

  it('sums everything else as before', () => {
    // Two runners with 1 and 4 asks and decisions, and 2 pull requests each.
    const counts = countsOf([runner('a', 3, 2), runner('b', 6, 2)], now)
    expect(counts?.total).toBe(1 + 4 + 2)
  })

  it('takes the larger set when the runners were asked about different numbers', () => {
    const counts = countsOf([runner('a', 1, 1), runner('b', 3, 3)], now)
    expect(counts?.total).toBe(3)
  })

  it('keeps today\'s sum for a runner too old to send the count', () => {
    // The old daemon's 3 includes pull requests it cannot name, so they stay.
    const counts = countsOf([runner('old', 3), runner('new', 3, 2)], now)
    expect(counts?.total).toBe(3 + 1 + 2)
    expect(countsOf([runner('old', 3, null), runner('older', 2)], now)?.total).toBe(5)
  })

  it('counts a runner with no pull requests as it always did', () => {
    expect(countsOf([runner('a', 2, 0), runner('b', 5, 0)], now)?.total).toBe(7)
  })

  it('never lets a pull-request count above the runner\'s own total subtract from the rest', () => {
    // A malformed pair is clamped to the runner's whole count.
    const counts = countsOf([runner('a', 1, 9), runner('b', 4, 0)], now)
    expect(counts?.total).toBe(4 + 1)
  })

  it('leaves a total of none as none', () => {
    expect(countsOf([runner('a', null, 2)], now)).toBeNull()
  })
})
