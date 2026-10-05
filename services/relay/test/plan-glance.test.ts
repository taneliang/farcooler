import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { planGlanceOf } from '../src/plan-glance'
import count from '../../../test/fixtures/contracts/notify/count.json'
import countPlan from '../../../test/fixtures/contracts/notify/count-plan.json'
import working from '../../../test/fixtures/contracts/notify/agent-working.json'
import pulseFixture from '../../../test/fixtures/contracts/pulse/plan.json'
import cardFixture from '../../../test/fixtures/contracts/live-activity/running/plan.json'

// The plan on the glance (ov-310): a runner's count notice carries each board
// with a plan, and the relay hands the one the glance draws to `/v1/pulse`
// (the widgets and the watch, which has no sockets) and to the Live Activity.
// These post what the daemon posts (the shared notice fixtures) and read what
// the apps read. Kept out of relay.test.ts, which is over its size budget.

const PULSE_TOKEN = '5d41402abc4b2a76b9719d911017c592ae2f6b0c8e3d1f7a9b4c6e8d0f2a4b6c'
const SAMPLED_AT = 1_791_019_800_000
const BOARD = countPlan.plan[0]

async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))
  return [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2, '0')).join('')
}

function post(path: string, body: unknown, bearer: string): Promise<Response> {
  return worker.fetch(
    new Request(`https://relay.test${path}`, {
      method: 'POST',
      headers: { authorization: `Bearer ${bearer}` },
      body: JSON.stringify(body),
    }),
    env as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
}

/// An account with a phone holding a pulse token, and a runner per token.
async function account(id: string, tokens: string[]) {
  await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES (?, ?)`).bind(id, Date.now()).run()
  for (const token of tokens) {
    await env.DB.prepare(
      `INSERT INTO daemons (id, account_id, token_hash, label, created_at) VALUES (?, ?, ?, 'Studio', ?)`,
    )
      .bind(crypto.randomUUID(), id, await sha256(token), Date.now())
      .run()
  }
  await env.DB.prepare(
    `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at, pulse_hash)
     VALUES (?, ?, 'apns', ?, 'Phone', ?, ?)`,
  )
    .bind(crypto.randomUUID(), id, `push-${id}`, Date.now(), await sha256(PULSE_TOKEN))
    .run()
}

function at(ms: number) {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(ms)
}

async function pulse(): Promise<Record<string, unknown>> {
  const response = await post('/v1/pulse', {}, PULSE_TOKEN)
  expect(response.status).toBe(200)
  return await response.json<Record<string, unknown>>()
}

beforeEach(async () => {
  vi.useRealTimers()
  vi.stubGlobal('fetch', async () => new Response('{}'))
  for (const table of ['install_cards', 'live_activities', 'devices', 'daemons', 'accounts']) {
    await env.DB.prepare(`DELETE FROM ${table}`).run()
  }
})

describe('the plan on the glance', () => {
  /// A runner that beats, as every runner new enough to send a plan does.
  async function beat(token: string, name = 'Studio') {
    expect((await post('/v1/heartbeat', { beatEvery: 300, name }, token)).status).toBe(200)
  }

  /// The board as the glance draws it: the runner's board, who said it, how
  /// long ago that runner was heard and whether it went quiet.
  const lead = (board: object, heardAgo: number, runner = 'Studio', quiet = false) =>
    ({ ...board, runner, heardAgo, ...(quiet ? { quiet: true } : {}) })

  it('answers the pulse with the board the daemon’s count notice carried', async () => {
    await account('user_1', ['mine'])
    at(SAMPLED_AT)
    await beat('mine')
    expect((await post('/v1/notify', countPlan, 'mine')).status).toBe(200)
    at(SAMPLED_AT + 90_000)
    expect(await pulse()).toEqual(pulseFixture)
    // Spelled out, not read back off the fixture: what the watch draws.
    expect(pulseFixture.plan).toEqual({
      workspace: 'Main',
      needsYou: 2,
      now: [{ name: 'mac-ux', state: 'review' }, { name: 'ov-310', state: 'building' }],
      next: 'mac-fu3',
      runner: 'Studio',
      heardAgo: 90_000,
    })
    vi.useRealTimers()
  })

  it('moves a running card to carry it, and the app decodes that card', async () => {
    await account('user_1', ['mine'])
    await env.DB.prepare(
      `INSERT INTO install_cards (id, account_id, update_token, environment, updated_at, pushed_at)
       VALUES (?, 'user_1', 'card-token', 'production', 0, 0)`,
    )
      .bind(crypto.randomUUID())
      .run()
    const cards: unknown[] = []
    vi.stubGlobal('fetch', async (input: any, init: any = {}) => {
      if (init.headers?.['apns-push-type'] === 'liveactivity') cards.push(JSON.parse(init.body))
      return new Response('{}')
    })
    at(SAMPLED_AT)
    await beat('mine')
    await post('/v1/notify', working, 'mine')
    expect((cards[0] as any).aps['content-state'].plan).toBeUndefined()
    await post('/v1/notify', countPlan, 'mine')
    expect(cards.length).toBe(2)
    expect(cards[1]).toEqual(cardFixture)
    expect((cards[1] as any).aps['content-state'].plan).toEqual(lead(BOARD, 0))

    // The same count and the same plan again move nothing the card shows,
    // and push nothing (review M3); a plan that moved does.
    await post('/v1/notify', countPlan, 'mine')
    expect(cards.length).toBe(2)
    await post('/v1/notify', { ...countPlan, plan: [{ ...BOARD, next: 'mac-fu4' }] }, 'mine')
    expect(cards.length).toBe(3)
    vi.useRealTimers()
  })

  it('keeps a plan a notice without one says nothing about, and drops it when the runner says none', async () => {
    await account('user_1', ['mine'])
    at(SAMPLED_AT)
    await beat('mine')
    await post('/v1/notify', countPlan, 'mine')
    await post('/v1/notify', count, 'mine')
    expect((await pulse()).plan).toEqual(lead(BOARD, 0))
    await post('/v1/notify', { ...count, plan: [] }, 'mine')
    expect(await pulse()).not.toHaveProperty('plan')
    vi.useRealTimers()
  })

  it('keeps the plan over a weekend nothing moved, with its runner still beating (review H1)', async () => {
    await account('user_1', ['mine'])
    at(SAMPLED_AT)
    await beat('mine')
    await post('/v1/notify', countPlan, 'mine')
    at(SAMPLED_AT + 3 * 24 * 3_600_000)
    await beat('mine')
    expect((await pulse()).plan).toEqual(lead(BOARD, 0))
    vi.useRealTimers()
  })

  it('draws a quiet runner’s last plan with its age, and a beating one outranks it (review H2)', async () => {
    await account('user_1', ['studio', 'laptop'])
    at(SAMPLED_AT)
    await beat('studio', 'Studio')
    await beat('laptop', 'Laptop')
    await post('/v1/notify', { ...countPlan, install: 'studio-install' }, 'studio')
    at(SAMPLED_AT + 1000)
    const laptop = { ...BOARD, workspace: 'Laptop board' }
    await post('/v1/notify', { ...countPlan, install: 'laptop-install', plan: [laptop] }, 'laptop')
    expect((await pulse()).plan).toEqual(lead(laptop, 0, 'Laptop'))

    // Sixteen minutes on, only the studio has beaten since: the laptop is
    // quiet, so the studio's plan leads.
    at(SAMPLED_AT + 16 * 60_000)
    await beat('studio', 'Studio')
    expect((await pulse()).plan).toEqual(lead(BOARD, 0))

    // Then the studio says it has no plan: the laptop's last word is all
    // there is, said with who and how long ago, never "no plan".
    await post('/v1/notify', { ...count, install: 'studio-install', plan: [] }, 'studio')
    expect((await pulse()).plan).toEqual(lead(laptop, 16 * 60_000 - 1000, 'Laptop', true))
    vi.useRealTimers()
  })

  it('leads with the board that needs the owner over a newer one that doesn’t (review M4)', async () => {
    await account('user_1', ['studio', 'laptop'])
    at(SAMPLED_AT)
    await beat('studio', 'Studio')
    await beat('laptop', 'Laptop')
    await post('/v1/notify', { ...countPlan, install: 'studio-install' }, 'studio')
    at(SAMPLED_AT + 1000)
    const calm = { ...BOARD, workspace: 'Ops', needsYou: 0 }
    await post('/v1/notify', { ...countPlan, install: 'laptop-install', plan: [calm] }, 'laptop')
    expect((await pulse()).plan).toEqual(lead(BOARD, 1000))
    vi.useRealTimers()
  })

  it('forgets an unpaired runner’s plan (review M2)', async () => {
    await account('user_1', ['mine'])
    await beat('mine')
    await post('/v1/notify', countPlan, 'mine')
    expect((await post('/v1/heartbeat', { withdrawn: true }, 'mine')).status).toBe(200)
    expect(await pulse()).not.toHaveProperty('plan')
    expect(await env.DB.prepare(`SELECT plan, plan_at FROM daemons`).first()).toEqual({ plan: null, plan_at: null })
  })

  it('keeps lane and board names and nothing else, within its bounds', () => {
    const lane = (name: string, state = 'building') => ({ name, state, reason: 'Card text' })
    const kept = planGlanceOf([
      {
        workspace: '  Main  ',
        needsYou: 1,
        now: [lane('a'), { name: 'b', state: 'Building!' }, lane('c'), lane('d')],
        next: 'e',
        story: 'A theme’s story',
        ask: 'Pick the accent',
      },
      { workspace: 'No count', now: [] },
      { workspace: 'Second', needsYou: 0, now: 'not a list' },
      { workspace: 'Third', needsYou: 0, now: [] },
      { workspace: 'Fourth', needsYou: 0, now: [] },
    ])
    expect(kept).toEqual([
      { workspace: 'Main', needsYou: 1, now: [{ name: 'a', state: 'building' }, { name: 'c', state: 'building' }], next: 'e' },
      { workspace: 'Second', needsYou: 0, now: [] },
      { workspace: 'Third', needsYou: 0, now: [] },
    ])
    expect(planGlanceOf({ not: 'a list' })).toBeNull()
    expect(planGlanceOf([{ workspace: 'W', needsYou: 0, now: [lane('x'.repeat(200))] }])![0].now[0].name)
      .toBe('x'.repeat(60))
    // A count past 32 bits would fail to decode on an arm64_32 watch.
    expect(planGlanceOf([{ workspace: 'W', needsYou: 2 ** 40, now: [] }])![0].needsYou).toBe(0x7fffffff)
  })
})
