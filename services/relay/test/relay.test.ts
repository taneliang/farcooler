import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'

/// What the relay must never get wrong.
///
/// This is the security boundary of the whole product and it had no tests at
/// all — CI proved the worker compiled and bundled, which is not the same as
/// proving that a stolen daemon token cannot name its own destination, or that
/// a WorkOS error body does not come back to the caller. Every test here is one
/// of those, not a coverage exercise.
///
/// Real D1, not a fake: the account scoping on revoke and the expiry predicate
/// on notify are SQL, and a hand-written fake would happily agree with whatever
/// this code believed.

const schema = [
  `CREATE TABLE IF NOT EXISTS accounts (
     id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, email TEXT)`,
  `CREATE TABLE IF NOT EXISTS devices (
     id TEXT PRIMARY KEY, account_id TEXT NOT NULL, platform TEXT NOT NULL,
     push_token TEXT NOT NULL, label TEXT NOT NULL, version TEXT,
     updated_at INTEGER NOT NULL, UNIQUE (platform, push_token))`,
  `CREATE TABLE IF NOT EXISTS daemons (
     id TEXT PRIMARY KEY, account_id TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE,
     label TEXT NOT NULL, version TEXT, created_at INTEGER NOT NULL,
     last_seen_at INTEGER, expires_at INTEGER)`,
]

async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))
  return [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2, '0')).join('')
}

function post(path: string, body: unknown, bearer?: string): Promise<Response> {
  return worker.fetch(
    new Request(`https://relay.test${path}`, {
      method: 'POST',
      headers: bearer ? { authorization: `Bearer ${bearer}` } : {},
      body: JSON.stringify(body),
    }),
    env as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
}

beforeEach(async () => {
  for (const statement of schema) await env.DB.prepare(statement).run()
  for (const table of ['devices', 'daemons', 'accounts']) {
    await env.DB.prepare(`DELETE FROM ${table}`).run()
  }
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('the shape of the API', () => {
  it('answers nothing but POST', async () => {
    const response = await worker.fetch(
      new Request('https://relay.test/v1/notify'),
      env as never,
      {} as never,
    )
    expect(response.status).toBe(405)
  })

  it('does not invent routes', async () => {
    expect((await post('/v1/nope', {})).status).toBe(404)
  })
})

describe('/v1/auth/token', () => {
  it('refuses an exchange with no PKCE verifier', async () => {
    // Without the verifier there is nothing binding the code to the app that
    // started sign-in, and `overnight://` is a scheme any app can claim.
    expect((await post('/v1/auth/token', { code: 'abc' })).status).toBe(400)
  })

  it('turns a WorkOS rejection into a 401 and leaks none of its body', async () => {
    // The body of a failed grant can echo the submitted credential back. The
    // top-level handler already refuses to return error text for this reason;
    // this path used to log the whole thing.
    vi.stubGlobal(
      'fetch',
      async () => new Response(JSON.stringify({ error: 'invalid_grant', code: 'SECRET' }), {
        status: 400,
      }),
    )
    const response = await post('/v1/auth/token', { code: 'abc', verifier: 'v' })
    expect(response.status).toBe(401)
    expect(await response.text()).not.toContain('SECRET')
  })

  it('creates the account on the way through', async () => {
    vi.stubGlobal(
      'fetch',
      async () => new Response(
        JSON.stringify({
          access_token: 'at',
          refresh_token: 'rt',
          user: { id: 'user_1', email: 'someone@example.test' },
        }),
      ),
    )
    const body = await (await post('/v1/auth/token', { code: 'abc', verifier: 'v' })).json<{
      accessToken: string
      userId: string
    }>()
    expect(body.accessToken).toBe('at')
    expect(body.userId).toBe('user_1')

    // Signing in is what creates the account, not registering a device. A
    // person who signs in and never grants push permission still exists.
    const account = await env.DB.prepare(`SELECT email FROM accounts WHERE id = ?`)
      .bind('user_1')
      .first<{ email: string }>()
    expect(account?.email).toBe('someone@example.test')
  })
})

describe('/v1/auth/refresh and /v1/auth/logout', () => {
  it('refuses a refresh with no token', async () => {
    expect((await post('/v1/auth/refresh', {})).status).toBe(400)
  })

  it('refuses a logout with no token', async () => {
    expect((await post('/v1/auth/logout', {})).status).toBe(400)
  })

  it('reports a logout as done even when WorkOS is unreachable', async () => {
    // The device is clearing its copy either way. An error here would only
    // teach the app to keep a credential when the server is having a bad day.
    vi.stubGlobal('fetch', async () => {
      throw new Error('network')
    })
    const response = await post('/v1/auth/logout', { refreshToken: 'rt' })
    expect(response.status).toBe(200)
  })
})

describe('/v1/notify', () => {
  async function pair(account: string, token: string, expiresAt: number | null = null) {
    await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES (?, ?)`)
      .bind(account, Date.now())
      .run()
    await env.DB.prepare(
      `INSERT INTO daemons (id, account_id, token_hash, label, created_at, expires_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    )
      .bind(crypto.randomUUID(), account, await sha256(token), 'Machine', Date.now(), expiresAt)
      .run()
  }

  it('refuses a request with no token', async () => {
    expect((await post('/v1/notify', { title: 'hi' })).status).toBe(401)
  })

  it('refuses a token nobody issued', async () => {
    await pair('user_1', 'good')
    expect((await post('/v1/notify', { title: 'hi' }, 'guessed')).status).toBe(401)
  })

  it('refuses a token that has expired', async () => {
    await pair('user_1', 'stale', Date.now() - 1000)
    expect((await post('/v1/notify', { title: 'hi' }, 'stale')).status).toBe(401)
  })

  it('still accepts a pairing made before expiries existed', async () => {
    // NULL means no expiry. Backfilling those would log people out of a feature
    // they had just set up, to introduce a policy.
    await pair('user_1', 'old', null)
    expect((await post('/v1/notify', { title: 'hi' }, 'old')).status).toBe(200)
  })

  it('will not deliver to an account other than its own', async () => {
    // The rule the whole design rests on: the daemon says "notify my user" and
    // never names a destination, so a stolen token is worth one thing —
    // notifying the phone of the person it was stolen from.
    await pair('user_1', 'mine')
    await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES (?, ?)`)
      .bind('user_2', Date.now())
      .run()
    await env.DB.prepare(
      `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    )
      .bind(crypto.randomUUID(), 'user_2', 'apns', 'someone-elses-phone', 'Their phone', Date.now())
      .run()

    const body = await (await post('/v1/notify', { title: 'hi' }, 'mine')).json<{
      delivered: number
    }>()
    expect(body.delivered).toBe(0)
  })

  it('records what the machine is running', async () => {
    await pair('user_1', 'mine')
    await post('/v1/notify', { title: 'hi', version: '0.2.0+abc123' }, 'mine')
    const row = await env.DB.prepare(`SELECT version, last_seen_at FROM daemons`).first<{
      version: string
      last_seen_at: number
    }>()
    expect(row?.version).toBe('0.2.0+abc123')
    expect(row?.last_seen_at).toBeGreaterThan(0)
  })

  it('needs something to say', async () => {
    await pair('user_1', 'mine')
    expect((await post('/v1/notify', {}, 'mine')).status).toBe(400)
  })
})

describe('the signed-in routes', () => {
  it('refuse a caller with no session', async () => {
    for (const path of ['/v1/devices', '/v1/daemons', '/v1/account', '/v1/devices/revoke']) {
      expect((await post(path, {})).status, path).toBe(401)
    }
  })
})
