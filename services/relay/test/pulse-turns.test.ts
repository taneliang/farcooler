import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import doneFailed from '../../../test/fixtures/contracts/notify/agent-done-failed.json'
import working from '../../../test/fixtures/contracts/notify/agent-working.json'
import pulseFixture from '../../../test/fixtures/contracts/pulse/turns.json'

// What `/v1/pulse` says about how each finished agent's turn ended (ov-239).
//
// The watch hears from the relay over this route and no other, so a failure
// the runner has since resolved can only clear there if the answer carries
// the outcome. These post what the daemon posts (the shared notice fixtures)
// and read what the watch reads. Kept out of relay.test.ts, which is over its
// size budget.

const PULSE_TOKEN = '5d41402abc4b2a76b9719d911017c592ae2f6b0c8e3d1f7a9b4c6e8d0f2a4b6c'
const SAMPLED_AT = 1_791_019_800_000

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

/// An account with a phone holding a pulse token, and a runner holding `token`.
async function account(id: string, token: string, pulse: string) {
  await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES (?, ?)`).bind(id, Date.now()).run()
  await env.DB.prepare(
    `INSERT INTO daemons (id, account_id, token_hash, label, created_at) VALUES (?, ?, ?, 'Studio', ?)`,
  )
    .bind(crypto.randomUUID(), id, await sha256(token), Date.now())
    .run()
  await env.DB.prepare(
    `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at, pulse_hash)
     VALUES (?, ?, 'apns', ?, 'Phone', ?, ?)`,
  )
    .bind(crypto.randomUUID(), id, `push-${id}`, Date.now(), await sha256(pulse))
    .run()
}

function at(ms: number) {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(ms)
}

async function turns(pulse = PULSE_TOKEN): Promise<unknown> {
  const response = await post('/v1/pulse', {}, pulse)
  expect(response.status).toBe(200)
  return (await response.json<{ turns?: unknown }>()).turns
}

beforeEach(async () => {
  vi.useRealTimers()
  vi.stubGlobal('fetch', async () => new Response('{}'))
  for (const table of ['install_cards', 'live_activities', 'devices', 'daemons', 'accounts']) {
    await env.DB.prepare(`DELETE FROM ${table}`).run()
  }
})

describe('the pulse answer’s turns', () => {
  it('names a finished agent’s outcome, and the quiet success after a failure replaces it', async () => {
    await account('user_1', 'mine', PULSE_TOKEN)
    at(SAMPLED_AT)
    expect((await post('/v1/notify', doneFailed, 'mine')).status).toBe(200)
    expect(await turns()).toEqual([{ terminal: 'term-01999a90aa10', failed: true, at: SAMPLED_AT }])

    at(SAMPLED_AT + 60_000)
    const quiet = { ...doneFailed, failed: false, alert: false }
    expect((await post('/v1/notify', quiet, 'mine')).status).toBe(200)
    expect(await turns()).toEqual([
      { terminal: 'term-01999a90aa10', failed: false, at: SAMPLED_AT + 60_000 },
    ])
    vi.useRealTimers()
  })

  it('leaves out an agent that went back to work, and one whose runner said nothing', async () => {
    await account('user_1', 'mine', PULSE_TOKEN)
    await post('/v1/notify', doneFailed, 'mine')
    const silent = { ...doneFailed, terminal: 'term-silent' }
    delete (silent as Record<string, unknown>).failed
    await post('/v1/notify', silent, 'mine')
    // Only the one that said: no word is not "finished well".
    expect(await turns()).toEqual([
      expect.objectContaining({ terminal: 'term-01999a90aa10', failed: true }),
    ])

    await post('/v1/notify', { ...working, terminal: doneFailed.terminal }, 'mine')
    expect(await turns()).toEqual([])
  })

  it('shows an account only its own agents', async () => {
    await account('user_1', 'mine', PULSE_TOKEN)
    await account('user_2', 'theirs', 'f'.repeat(64))
    await post('/v1/notify', doneFailed, 'theirs')
    expect(await turns()).toEqual([])
  })

  it('answers the shared fixture the watch’s tests decode', async () => {
    await account('user_1', 'mine', PULSE_TOKEN)
    at(SAMPLED_AT)
    await post('/v1/notify', doneFailed, 'mine')
    at(SAMPLED_AT + 60_000)
    await post('/v1/notify', { ...doneFailed, failed: false, alert: false }, 'mine')
    const response = await post('/v1/pulse', {}, PULSE_TOKEN)
    expect(await response.json()).toEqual(pulseFixture)
    vi.useRealTimers()
  })
})
