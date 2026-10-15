import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { pruneSessionsSeen } from '../src/session-prune'

// Relay session hygiene (ov-567, ov-568); the helpers are signout-all's.
// "Sign out everywhere" (ov-564): the old token is refused afterwards, another
// account is untouched, and the sessions are ended at WorkOS too. Against the
// real migration chain in workerd's D1: the cutoff is a column and a
// comparison, and a fake of either would agree with the code.

const signing = (await crypto.subtle.generateKey(
  { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256' },
  true,
  ['sign', 'verify'],
)) as CryptoKeyPair
const publicJwk = { ...(await crypto.subtle.exportKey('jwk', signing.publicKey)), kid: 'test-key' }

function base64Url(value: string | Uint8Array): string {
  const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value
  return btoa(String.fromCharCode(...bytes)).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
}

/// A token for `user`, minted `ageSeconds` ago (negative: that far ahead, which
/// the verifier's clock leeway allows, and which stands in for a sign-in that
/// happens after the sign-out).
async function sessionFor(user: string, sid: string | null = 'sid_1', ageSeconds = 0): Promise<string> {
  const now = Math.floor(Date.now() / 1000) - ageSeconds
  const claims = { sub: user, iss: (env as any).WORKOS_ISSUER, client_id: (env as any).WORKOS_CLIENT_ID, ...(sid === null ? {} : { sid }), iat: now, exp: now + 3600 }
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const payload = base64Url(JSON.stringify(claims))
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', signing.privateKey, new TextEncoder().encode(`${header}.${payload}`))
  return `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
}

let calls: { url: string; body: string | null }[] = []
let authenticateOk = true

beforeEach(async () => {
  calls = []
  authenticateOk = true
  await env.DB.prepare(`DELETE FROM accounts`).run()
  vi.stubGlobal('fetch', async (input: any, init?: any) => {
    const url = typeof input === 'string' ? input : input.url
    calls.push({ url, body: init?.body ?? null })
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    if (url.endsWith('/user_management/authenticate')) {
      // The stand-in refreshes: the new access token carries the session's sid.
      return authenticateOk
        ? new Response(JSON.stringify({ access_token: await sessionFor('alice', 'sid_real'), refresh_token: 'rt2' }))
        : new Response('{}', { status: 400 })
    }
    return new Response('{}')
  })
})

async function post(path: string, body: unknown): Promise<number> {
  const response = await worker.fetch(
    new Request(`https://relay.test${path}`, { method: 'POST', body: JSON.stringify(body) }),
    env as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
  return response.status
}

const revokes = () => calls.filter(c => c.url.endsWith('/user_management/sessions/revoke')).map(c => JSON.parse(c.body!).session_id)

describe('logout ends the WorkOS session (ov-567)', () => {
  it('revokes by the session’s sid, never by the refresh token', async () => {
    expect(await post('/v1/auth/logout', { refreshToken: 'rt-secret' })).toBe(200)
    expect(revokes()).toEqual(['sid_real'])
  })

  it('still answers ok, and revokes nothing, when the refresh token no longer works', async () => {
    authenticateOk = false
    expect(await post('/v1/auth/logout', { refreshToken: 'rt-dead' })).toBe(200)
    expect(revokes()).toEqual([])
  })
})

const DAY = 24 * 60 * 60 * 1000
const NOW = Date.UTC(2026, 9, 15)

async function seed(account: string, revokedAt: number | null, rows: [string, number][]) {
  await env.DB.prepare(`INSERT OR REPLACE INTO accounts (id, created_at, sessions_revoked_at) VALUES (?, 1, ?)`).bind(account, revokedAt).run()
  for (const [sid, firstSeen] of rows) {
    await env.DB.prepare(`INSERT INTO sessions_seen (sid, account_id, first_seen) VALUES (?, ?, ?)`).bind(sid, account, firstSeen).run()
  }
}
const sids = async () =>
  ((await env.DB.prepare(`SELECT account_id || ':' || sid AS k FROM sessions_seen ORDER BY k`).all<{ k: string }>()).results ?? []).map(r => r.k)

describe('sessions_seen is pruned (ov-568)', () => {
  it('drops rows older than the lifetime and keeps younger ones, from the cron', async () => {
    await seed('alice', null, [['old', NOW - 401 * DAY], ['young', NOW - 399 * DAY]])
    vi.useFakeTimers({ toFake: ['Date'], now: NOW })
    try {
      await worker.scheduled!({ cron: '*/5 * * * *', scheduledTime: NOW, noRetry() {} } as never, env as never, {} as never)
    } finally {
      vi.useRealTimers()
    }
    expect(await sids()).toEqual(['alice:young'])
  })

  it('keeps an old row while the account’s cutoff, still inside the lifetime, covers it', async () => {
    await seed('alice', NOW - 10 * DAY, [['covered', NOW - 450 * DAY]])
    await seed('bob', NOW - 450 * DAY, [['dead', NOW - 460 * DAY]])
    await seed('carol', null, [['plain', NOW - 450 * DAY]])
    await pruneSessionsSeen(env as never, NOW)
    expect(await sids()).toEqual(['alice:covered'])
  })
})
