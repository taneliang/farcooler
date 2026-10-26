import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import wranglerToml from '../wrangler.toml?raw'

// `directory/me` has a per-account limit (ov-815, SF2 of the ov-811 review):
// the web client's Worker calls it on every page load and every
// /api/relay/directory/me, and each call is several D1 reads. The limit keys
// on the account the token verifies as, and is spent before any of them.

const signing = (await crypto.subtle.generateKey(
  { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256' },
  true,
  ['sign', 'verify'],
)) as CryptoKeyPair
const stranger = (await crypto.subtle.generateKey(
  { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256' },
  true,
  ['sign', 'verify'],
)) as CryptoKeyPair
const publicJwk = { ...(await crypto.subtle.exportKey('jwk', signing.publicKey)), kid: 'test-key' }

function base64Url(value: string | Uint8Array): string {
  const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value
  return btoa(String.fromCharCode(...bytes)).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
}

async function sessionFor(user: string, key: CryptoKey = signing.privateKey): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const claims = { sub: user, iss: (env as any).WORKOS_ISSUER, client_id: (env as any).WORKOS_CLIENT_ID, sid: `sid_${user}`, iat: now, exp: now + 3600 }
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const payload = base64Url(JSON.stringify(claims))
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', key, new TextEncoder().encode(`${header}.${payload}`))
  return `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
}

/// A limiter like Cloudflare's, with a fixed window that never ends: `limit`
/// calls per key, then refused. `asked` is every key it was asked about.
function limiter(limit: number) {
  const spent = new Map<string, number>()
  const asked: string[] = []
  return {
    asked,
    async limit({ key }: { key: string }) {
      asked.push(key)
      const n = (spent.get(key) ?? 0) + 1
      spent.set(key, n)
      return { success: n <= limit }
    },
  }
}

/// The real D1, counting every statement prepared or batched against it.
function counted() {
  const db = (env as any).DB as D1Database
  const seen = { statements: 0 }
  const proxy = new Proxy(db, {
    get(target, prop) {
      if (prop === 'prepare') return (sql: string) => (seen.statements++, target.prepare(sql))
      if (prop === 'batch') return (statements: D1PreparedStatement[]) => target.batch(statements)
      const value = (target as any)[prop]
      return typeof value === 'function' ? value.bind(target) : value
    },
  })
  return { db: proxy, seen }
}

async function me(token: string, extra: Record<string, unknown>): Promise<Response> {
  return await worker.fetch(
    new Request('https://relay.test/v1/directory/me', { method: 'POST', headers: { authorization: `Bearer ${token}` }, body: '{}' }),
    { ...env, ...extra } as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
}

beforeEach(() => {
  vi.stubGlobal('fetch', async () => new Response(JSON.stringify({ keys: [publicJwk] })))
})

describe('directory/me per account', () => {
  it("leaves another account reading after one account has used up its limit", async () => {
    const limit = limiter(3)
    const alice = await sessionFor('user_alice')
    const bob = await sessionFor('user_bob')
    for (let i = 0; i < 3; i++) expect((await me(alice, { DIRECTORY_ME_LIMIT: limit })).status).toBe(200)
    for (let i = 0; i < 5; i++) {
      const refused = await me(alice, { DIRECTORY_ME_LIMIT: limit })
      expect(refused.status).toBe(429)
      expect(await refused.json()).toEqual({ error: 'slow down' })
    }
    const answer = await me(bob, { DIRECTORY_ME_LIMIT: limit })
    expect(answer.status).toBe(200)
    expect((await answer.json<any>()).user.id).toBe('user_bob')
    // Keyed on the account the token verified as: not the address, not the token.
    expect(new Set(limit.asked)).toEqual(new Set(['user_alice', 'user_bob']))
  })

  it('refuses before reading the database', async () => {
    const limit = limiter(1)
    const token = await sessionFor('user_reads')
    const { db, seen } = counted()
    expect((await me(token, { DIRECTORY_ME_LIMIT: limit, DB: db })).status).toBe(200)
    expect(seen.statements).toBeGreaterThan(0)
    seen.statements = 0
    expect((await me(token, { DIRECTORY_ME_LIMIT: limit, DB: db })).status).toBe(429)
    expect(seen.statements).toBe(0)
  })

  it("spends nothing of an account's limit on a token that doesn't verify", async () => {
    // Anyone can write `sub: user_bob` into a token; only a verified one counts,
    // or a stranger could lock bob out by naming him.
    const limit = limiter(1)
    const forged = await sessionFor('user_bob', stranger.privateKey)
    for (let i = 0; i < 3; i++) expect((await me(forged, { DIRECTORY_ME_LIMIT: limit })).status).toBe(401)
    expect(limit.asked).toEqual([])
    expect((await me(await sessionFor('user_bob'), { DIRECTORY_ME_LIMIT: limit })).status).toBe(200)
  })

  it('is open with no binding, as a local relay has none', async () => {
    expect((env as any).DIRECTORY_ME_LIMIT).toBeUndefined()
    const token = await sessionFor('user_open')
    for (let i = 0; i < 5; i++) expect((await me(token, {})).status).toBe(200)
  })

  it('is throttled on every channel, generously, in a namespace of its own', () => {
    const blocks = wranglerToml.split(/^\[\[(?:env\.([a-z]+)\.)?unsafe\.bindings\]\]$/m)
    const channels: string[] = []
    const ids: string[] = []
    for (let i = 1; i < blocks.length; i += 2) {
      const block = blocks[i + 1]
      if (!/name = "DIRECTORY_ME_LIMIT"/.test(block)) continue
      channels.push(blocks[i] ?? 'stable')
      expect(block).toContain('type = "ratelimit"')
      expect(block).toContain('simple = { limit = 120, period = 60 }')
      ids.push(block.match(/namespace_id = "(\d+)"/)![1])
    }
    expect(channels.sort()).toEqual(['canary', 'local', 'preview', 'stable'])
    expect(ids.sort()).toEqual(['1021', '1022', '1023', '1024'])
    const namespaces = [...wranglerToml.matchAll(/namespace_id = "(\d+)"/g)].map(m => m[1])
    expect(new Set(namespaces).size).toBe(namespaces.length)
  })
})
