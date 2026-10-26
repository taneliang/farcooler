import { env } from 'cloudflare:test'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'

// `directory/me` is what the web client's Worker calls on every page load and
// every /api/relay/directory/me (ov-811, S2). Anyone can make an account, so
// whatever it writes, anyone can make the relay write in a loop. After the
// first call it writes nothing: the rows it would write are already there.
//
// Triggers count the writes, in the real D1: a SQL upsert that "changes
// nothing" still writes its row, and only the database can say so.

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

async function sessionFor(user: string, extra: Record<string, unknown> = {}): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const claims = { sub: user, iss: (env as any).WORKOS_ISSUER, client_id: (env as any).WORKOS_CLIENT_ID, sid: 'sid_me', iat: now, exp: now + 3600, ...extra }
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const payload = base64Url(JSON.stringify(claims))
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', signing.privateKey, new TextEncoder().encode(`${header}.${payload}`))
  return `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
}

const TABLES = ['accounts', 'sessions_seen', 'teams', 'memberships']

beforeEach(async () => {
  vi.stubGlobal('fetch', async () => new Response(JSON.stringify({ keys: [publicJwk] })))
  await env.DB.prepare(`CREATE TABLE IF NOT EXISTS test_writes (tbl TEXT NOT NULL)`).run()
  for (const table of TABLES) {
    for (const event of ['INSERT', 'UPDATE', 'DELETE']) {
      await env.DB.prepare(
        `CREATE TRIGGER IF NOT EXISTS test_${table}_${event} AFTER ${event} ON ${table} BEGIN INSERT INTO test_writes VALUES ('${table}'); END`,
      ).run()
    }
  }
  await env.DB.prepare(`DELETE FROM test_writes`).run()
})

afterEach(async () => {
  for (const table of TABLES) {
    for (const event of ['INSERT', 'UPDATE', 'DELETE']) await env.DB.prepare(`DROP TRIGGER IF EXISTS test_${table}_${event}`).run()
  }
  await env.DB.prepare(`DROP TABLE IF EXISTS test_writes`).run()
})

async function me(token: string): Promise<Response> {
  return await worker.fetch(
    new Request('https://relay.test/v1/directory/me', { method: 'POST', headers: { authorization: `Bearer ${token}` }, body: '{}' }),
    env as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
}

async function writes(): Promise<string[]> {
  const { results } = await env.DB.prepare(`SELECT tbl FROM test_writes ORDER BY rowid`).all<{ tbl: string }>()
  await env.DB.prepare(`DELETE FROM test_writes`).run()
  return results.map(row => row.tbl)
}

describe('directory/me writes nothing it has written before', () => {
  it('makes the account, its session and its personal team once, then only reads', async () => {
    const token = await sessionFor('user_writes')
    const first = await me(token)
    expect(first.status).toBe(200)
    expect((await writes()).sort()).toEqual(['accounts', 'memberships', 'sessions_seen', 'teams'])

    for (let i = 0; i < 5; i++) expect((await me(token)).status).toBe(200)
    expect(await writes()).toEqual([])

    const body = await (await me(token)).json<any>()
    expect(body.user.id).toBe('user_writes')
    expect(body.teams.map((t: any) => t.personal)).toEqual([true])
  })

  it('still writes an email the token carries that the account lacks, once', async () => {
    await me(await sessionFor('user_mail'))
    await writes()
    const withMail = await sessionFor('user_mail', { email: 'new@example.test' })
    await me(withMail)
    expect(await writes()).toEqual(['accounts'])
    await me(withMail)
    expect(await writes()).toEqual([])
    const row = await env.DB.prepare(`SELECT email FROM accounts WHERE id = ?`).bind('user_mail').first<{ email: string }>()
    expect(row?.email).toBe('new@example.test')
  })

  it('still refuses a session from before Sign Out Everywhere when it skips the write', async () => {
    // The read that replaced the upsert's RETURNING must still see the stamp.
    const token = await sessionFor('user_out')
    expect((await me(token)).status).toBe(200)
    await env.DB.prepare(`UPDATE accounts SET sessions_revoked_at = ? WHERE id = ?`).bind(Date.now() + 5000, 'user_out').run()
    expect((await me(token)).status).toBe(401)
  })
})
