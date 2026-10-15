import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'

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
let workos: { sessions: { id: string; status: string }[]; listOk: boolean; revokeOk: boolean }

beforeEach(async () => {
  calls = []
  workos = { sessions: [{ id: 'sid_1', status: 'active' }, { id: 'sid_phone', status: 'active' }, { id: 'sid_old', status: 'revoked' }], listOk: true, revokeOk: true }
  await env.DB.prepare(`DELETE FROM accounts`).run()
  vi.stubGlobal('fetch', async (input: any, init?: any) => {
    const url = typeof input === 'string' ? input : input.url
    calls.push({ url, body: init?.body ?? null })
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    if (url.includes('/user_management/users/') && url.includes('/sessions')) {
      return workos.listOk ? new Response(JSON.stringify({ data: workos.sessions, list_metadata: {} })) : new Response('{}', { status: 500 })
    }
    if (url.endsWith('/user_management/sessions/revoke')) return new Response('{}', { status: workos.revokeOk ? 200 : 500 })
    return new Response('{}')
  })
})

async function post(path: string, token: string | null, body: unknown = {}): Promise<{ status: number; body: any }> {
  const response = await worker.fetch(
    new Request(`https://relay.test${path}`, {
      method: 'POST',
      headers: token ? { authorization: `Bearer ${token}` } : {},
      body: JSON.stringify(body),
    }),
    env as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
  return { status: response.status, body: await response.json() }
}

/// Any route that needs a session.
const reads = (token: string | null) => post('/v1/account', token)
const signOutAll = (token: string | null, body: unknown = {}) => post('/v1/account/signout-all', token, body)

/// Past the clock's second, so a token minted next is later than the sign-out.
const AFTER = -5

describe('sign out everywhere', () => {
  it('refuses the old token afterwards, and a later sign-in works', async () => {
    const old = await sessionFor('alice')
    expect((await reads(old)).status).toBe(200)

    const answer = await signOutAll(old)
    expect(answer.status).toBe(200)
    expect(answer.body).toEqual({ ok: true, complete: true, ended: 2, runners: [] })

    expect((await reads(old)).status).toBe(401)
    expect((await reads(await sessionFor('alice', 'sid_2', 30))).status).toBe(401)
    expect((await reads(await sessionFor('alice', 'sid_3', AFTER))).status).toBe(200)
  })

  it('refuses another copy of the account’s tokens, on a directory route too', async () => {
    const phone = await sessionFor('alice', 'sid_phone')
    await signOutAll(await sessionFor('alice'))
    expect((await post('/v1/directory/me', phone)).status).toBe(401)
  })

  it('leaves another account alone', async () => {
    const bob = await sessionFor('bob', 'sid_bob')
    expect((await reads(bob)).status).toBe(200)
    await signOutAll(await sessionFor('alice'))
    expect((await reads(bob)).status).toBe(200)
    const row = await env.DB.prepare(`SELECT sessions_revoked_at FROM accounts WHERE id = 'bob'`).first<{ sessions_revoked_at: number | null }>()
    expect(row?.sessions_revoked_at).toBeNull()
    expect(calls.filter(c => c.url.includes('/users/bob'))).toEqual([])
  })

  it('needs a session', async () => {
    expect((await signOutAll(null)).status).toBe(401)
    expect((await signOutAll('not.a.token')).status).toBe(401)
  })

  it('ends each live session at WorkOS, and not the ones already ended', async () => {
    await signOutAll(await sessionFor('alice'))
    const revoked = calls
      .filter(c => c.url.endsWith('/user_management/sessions/revoke'))
      .map(c => JSON.parse(c.body!).session_id)
      .sort()
    expect(revoked).toEqual(['sid_1', 'sid_phone'])
  })

  it('still ends every access token when WorkOS can’t be reached, and says it is not complete', async () => {
    workos.listOk = false
    workos.revokeOk = false
    const old = await sessionFor('alice')
    const answer = await signOutAll(old)
    expect(answer.status).toBe(200)
    expect(answer.body).toEqual({ ok: true, complete: false, ended: 0, runners: [] })
    expect((await reads(old)).status).toBe(401)
  })

  it('writes an audit line', async () => {
    await signOutAll(await sessionFor('alice'))
    const line = await env.DB.prepare(`SELECT actor, action, team_id FROM audit WHERE action = 'sessions.revoked_all'`).first()
    expect(line).toEqual({ actor: 'alice', action: 'sessions.revoked_all', team_id: null })
  })

  it('tells the account’s other apps, and not the one signing out', async () => {
    await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES ('alice', 1)`).run()
    for (const [id, token] of [['d1', 'tok-here'], ['d2', 'tok-other']]) {
      await env.DB.prepare(
        `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at) VALUES (?, 'alice', 'android', ?, 'Phone', 1)`,
      ).bind(id, token).run()
    }
    await signOutAll(await sessionFor('alice'), { pushToken: 'tok-here' })
    const pushes = calls.filter(c => c.url.includes('fcm.googleapis.com')).map(c => c.body ?? '')
    expect(pushes.some(b => b.includes('tok-other'))).toBe(true)
    expect(pushes.some(b => b.includes('tok-here'))).toBe(false)
  })

  it('a second call still works for the retry, from a token minted after the first', async () => {
    await signOutAll(await sessionFor('alice'))
    const again = await signOutAll(await sessionFor('alice', 'sid_2', AFTER))
    expect(again.status).toBe(200)
  })

  /// H1: a refresh keeps the `sid` and mints a new `iat`, so the stamp alone
  /// can't stop one.
  it('refuses a refreshed token of a session the relay had seen, with WorkOS answering', async () => {
    expect((await reads(await sessionFor('alice', 'sid_phone'))).status).toBe(200)
    await signOutAll(await sessionFor('alice'))
    expect((await reads(await sessionFor('alice', 'sid_phone', AFTER))).status).toBe(401)
  })

  it('refuses a refreshed token of a session the relay had seen, with WorkOS down', async () => {
    workos.listOk = false
    workos.revokeOk = false
    expect((await reads(await sessionFor('alice', 'sid_phone'))).status).toBe(200)
    const answer = await signOutAll(await sessionFor('alice'))
    expect(answer.body.complete).toBe(false)
    expect((await reads(await sessionFor('alice', 'sid_phone', AFTER))).status).toBe(401)
  })

  it('refuses a refreshed token of a session WorkOS lists that the relay never saw', async () => {
    await signOutAll(await sessionFor('alice'))
    expect((await reads(await sessionFor('alice', 'sid_phone', AFTER))).status).toBe(401)
    // A session that signs in afterwards is a new one.
    expect((await reads(await sessionFor('alice', 'sid_new', AFTER))).status).toBe(200)
  })

  it('draws the sid line at the stamp: first seen at it is refused, after it is not', async () => {
    await reads(await sessionFor('alice', 'sid_a'))
    await env.DB.prepare(`UPDATE accounts SET sessions_revoked_at = 5000 WHERE id = 'alice'`).run()
    await env.DB.prepare(`INSERT INTO sessions_seen (sid, account_id, first_seen) VALUES ('at', 'alice', 5000), ('after', 'alice', 5001)`).run()
    expect((await reads(await sessionFor('alice', 'at', AFTER))).status).toBe(401)
    expect((await reads(await sessionFor('alice', 'after', AFTER))).status).toBe(200)
  })

  it('draws the iat line at the stamp plus the margin, for a token with no known session', async () => {
    // Frozen mid-second: `now` and the tokens' `iat` must come from the same
    // second, and a real clock can tick over between the two reads.
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(Math.floor(Date.now() / 1000) * 1000 + 500)
    try {
      const now = Math.floor(Date.now() / 1000)
      const stamp = (ms: number) => env.DB.prepare(`UPDATE accounts SET sessions_revoked_at = ? WHERE id = 'alice'`).bind(ms).run()
      await reads(await sessionFor('alice', 'sid_a'))
      await stamp(now * 1000 - 2000)
      expect((await reads(await sessionFor('alice', 'sid_b'))).status).toBe(401)
      await stamp(now * 1000 - 2001)
      expect((await reads(await sessionFor('alice', 'sid_c'))).status).toBe(200)
    } finally {
      vi.useRealTimers()
    }
  })

  it('stamps again after the WorkOS calls, so a session first seen during them is refused', async () => {
    // A frozen clock, moved by hand: with the real one, the racer's first
    // request can land in the same millisecond as the first stamp (a fast
    // machine, two D1 writes and a stubbed fetch apart), and a session first
    // seen AT the stamp is refused already, so the `during` check failed about
    // one run in five.
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(Date.now())
    try {
      let during: string | null = null
      const base = (globalThis as any).fetch
      vi.stubGlobal('fetch', async (input: any, init?: any) => {
        const url = typeof input === 'string' ? input : input.url
        if (url.includes('/sessions/revoke')) {
          // A session the relay first sees while sign-out is still talking to
          // WorkOS: strictly after the first stamp.
          vi.setSystemTime(Date.now() + 10)
          during = (await reads(await sessionFor('alice', 'sid_racer', AFTER))).status.toString()
        }
        return base(input, init)
      })
      await signOutAll(await sessionFor('alice'))
      expect(during).toBe('200')
      expect((await reads(await sessionFor('alice', 'sid_racer', AFTER))).status).toBe(401)
    } finally {
      vi.useRealTimers()
    }
  })

  it('refuses a token with no sid, which only its iat could cut off', async () => {
    expect((await reads(await sessionFor('alice', null))).status).toBe(401)
    expect((await reads(await sessionFor('alice', 'sid_ok'))).status).toBe(200)
  })

  /// H2: credentials that don't need a session.
  it('removes push rows, pulse tokens and Live Activity cards, which no session guards', async () => {
    await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES ('alice', 1)`).run()
    const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode('pulse-token')))]
      .map(b => b.toString(16).padStart(2, '0')).join('')
    await env.DB.prepare(
      `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at, pulse_hash) VALUES ('d1', 'alice', 'android', 'tok-1', 'Phone', 1, ?)`,
    ).bind(hash).run()
    await env.DB.prepare(`INSERT INTO install_cards (id, account_id, update_token, updated_at) VALUES ('c1', 'alice', 'u', 1)`).run()
    await env.DB.prepare(
      `INSERT INTO live_activities (id, account_id, terminal, update_token, updated_at) VALUES ('l1', 'alice', 't', 'u', 1)`,
    ).run()
    const pulse = () => worker.fetch(new Request('https://relay.test/v1/pulse', { method: 'POST', headers: { authorization: 'Bearer pulse-token' }, body: '{}' }), env as never, { waitUntil() {}, passThroughOnException() {} } as never)
    expect((await pulse()).status).toBe(200)

    await signOutAll(await sessionFor('alice'))

    expect((await pulse()).status).toBe(401)
    for (const table of ['devices', 'install_cards', 'live_activities']) {
      const row = await env.DB.prepare(`SELECT COUNT(*) AS n FROM ${table} WHERE account_id = 'alice'`).first<{ n: number }>()
      expect(row?.n, table).toBe(0)
    }
  })

  it('still sends the notice before the push rows go', async () => {
    await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES ('alice', 1)`).run()
    await env.DB.prepare(
      `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at) VALUES ('d2', 'alice', 'android', 'tok-other', 'Phone', 1)`,
    ).run()
    await signOutAll(await sessionFor('alice'))
    expect(calls.some(c => c.url.includes('fcm.googleapis.com') && (c.body ?? '').includes('tok-other'))).toBe(true)
  })

  /// M1: pairings stay, and the person is told which are recent.
  it('keeps runner pairings and lists the ones paired since the last sign out everywhere', async () => {
    await env.DB.prepare(`INSERT INTO accounts (id, created_at, sessions_revoked_at) VALUES ('alice', 1, 1000)`).run()
    const pair = (id: string, label: string, at: number) =>
      env.DB.prepare(`INSERT INTO daemons (id, account_id, token_hash, label, created_at) VALUES (?, 'alice', ?, ?, ?)`).bind(id, `h-${id}`, label, at).run()
    await pair('old', 'Old Box', 500)
    await pair('new', 'Dev “Box”', Date.now() - 1000)
    const answer = await signOutAll(await sessionFor('alice'))
    expect(answer.body.runners).toEqual([{ id: 'new', label: 'Dev Box', pairedAt: expect.any(Number) }])
    const kept = await env.DB.prepare(`SELECT COUNT(*) AS n FROM daemons WHERE account_id = 'alice'`).first<{ n: number }>()
    expect(kept?.n).toBe(2)
  })

  /// Low 2: a route that verified a token itself would skip both tests.
  it('has only session.ts call verifySession', () => {
    const sources = import.meta.glob('../src/**/*.ts', { query: '?raw', import: 'default', eager: true }) as Record<string, string>
    const callers = Object.entries(sources)
      .filter(([, text]) => /\bverifySession\(/.test(text))
      .map(([path]) => path.replace('../src/', ''))
      .filter(path => path !== 'workos.ts')
    expect(callers).toEqual(['session.ts'])
  })
})
