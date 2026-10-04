import { describe, expect, it } from 'vitest'

// Vite resolves this above the package root because `vitest.config.ts` allows
// the repository (`server.fs.allow`), for the same reason it allows the
// Android sources: a drift between two languages is only seen by a test that
// reads both.
import table from '../../../test/fixtures/fleet-composition.json'
import { composeFleet, countsOf, quietOf, type AgentRow, type Machine } from '../src/index'

// The shared fleet table (ov-166): the card's composition, held to the same
// cases as the app's widgets (`FleetCompositionTests.swift` in AgentKit). See
// the table's `_about` for what each field means.

interface Runner {
  id: string
  needsYou: number | null
  /// Worktrees to review, as the runner sent them; absent or null when it sent none.
  reviews?: number | null
  quiet: boolean
}

interface Agent {
  terminal: string
  runner: string
  status: string
  sinceS: number
  sinceMs?: number
  heardS: number
}

interface Case {
  name: string
  runners: Runner[]
  agents: Agent[]
  expect: {
    order: string[]
    blocked: number
    review: number
    reviewsWaiting?: number | null
    working: number
    needsYou: number | null
    header: number
    shown: string[]
  }
}

const now: number = table.now
const cases = table.cases as Case[]

/// A runner as `readFleet` reads it: one token, beating every five minutes,
/// heard just now, or twenty minutes ago when it's quiet (`quietAfterMs` is
/// fifteen at that beat).
function machine(runner: Runner): Machine {
  return {
    id: runner.id,
    label: runner.id,
    name: runner.id,
    install_id: null,
    needs_you: runner.needsYou,
    needs_you_at: runner.needsYou === null ? null : now,
    reviews: runner.reviews ?? null,
    last_seen_at: runner.quiet ? now - 20 * 60 * 1000 : now,
    beat_every: 300,
    expires_at: null,
  }
}

/// An agent as `live_activities` holds it, written by its runner's token.
function row(agent: Agent): AgentRow {
  return {
    terminal: agent.terminal,
    label: agent.terminal,
    machine: agent.runner,
    daemon_id: agent.runner,
    workspace: null,
    status: agent.status,
    detail: null,
    insertions: null,
    deletions: null,
    commits: null,
    trace: null,
    trace_anchor: null,
    started_at: null,
    status_since: now - (agent.sinceMs ?? agent.sinceS * 1000),
    updated_at: now - agent.heardS * 1000,
    ask_id: null,
    ask_tool: null,
    ask_until: null,
  }
}

describe('the shared fleet table', () => {
  it('has cases', () => {
    // A table that failed to load as an empty list would pass every case below.
    expect(cases.length).toBeGreaterThan(5)
  })

  for (const each of cases) {
    it(each.name, () => {
      const machines = each.runners.map(machine)
      // Shuffled the way `all()` may hand them over: reversed from the table.
      const rows = each.agents.map(row).reverse()
      const fleet = composeFleet(rows, countsOf(machines, now), now, quietOf(machines, rows, now))

      expect(fleet.all.map(each => each.terminal)).toEqual(each.expect.order)
      expect(fleet.blocked).toBe(each.expect.blocked)
      expect(fleet.review).toBe(each.expect.review)
      // The card's "to review" and the app's `reviewsWaiting` count worktrees
      // (ov-181): the same number whenever every runner counted them, which is
      // every case that names `reviewsWaiting` but the one that mixes in a
      // runner that did not.
      if (each.runners.length > 0 && each.runners.every(runner => runner.reviews != null)) {
        expect(fleet.review).toBe(each.expect.reviewsWaiting)
      }
      expect(fleet.working).toBe(each.expect.working)
      expect(fleet.needsYou).toBe(each.expect.needsYou)
      // `fleetHeader`'s and the card's `headerCount`: the count when any
      // runner sent one, else the blocked agents.
      expect(fleet.needsYou ?? fleet.blocked).toBe(each.expect.header)
      expect(fleet.shown.map(each => each.terminal)).toEqual(each.expect.shown)
    })
  }
})
