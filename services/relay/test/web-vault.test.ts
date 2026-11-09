import { env } from 'cloudflare:test'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { CREDENTIAL_MAX, VAULTS_MAX, VAULT_MAX } from '../src/web-vault'

// A browser's sealed copy (ov-779): only its own account reads or writes it,
// it holds ciphertext and nothing in the clear, and there is a ceiling on how
// much one account keeps. Against the real migration chain in workerd's D1.

const signing = (await crypto.subtle.generateKey(
  { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256' },
  true,
  ['sign', 'verify'],
)) as CryptoKeyPair
const publicJwk = { ...(await crypto.subtle.exportKey('jwk', signing.publicKey)), kid: 'test-key' }

function base64Url(value: string | Uint8Array): string {
  const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
}

async function sessionFor(user: string): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const claims = { sub: user, iss: (env as any).WORKOS_ISSUER, client_id: (env as any).WORKOS_CLIENT_ID, sid: `sid_${user}`, iat: now, exp: now + 3600 }
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const payload = base64Url(JSON.stringify(claims))
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', signing.privateKey, new TextEncoder().encode(`${header}.${payload}`))
  return `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
}

const ctx = { waitUntil() {}, passThroughOnException() {} } as never

async function call(method: string, user: string | null, body?: string, path = '/v1/web/vault'): Promise<{ status: number; body: any; headers: Headers }> {
  const response = await worker.fetch(
    new Request(`https://relay.test${path}`, { method, headers: user ? { authorization: `Bearer ${await sessionFor(user)}` } : {}, body }),
    env as never,
    ctx,
  )
  return { status: response.status, body: await response.json(), headers: response.headers }
}

/// Ciphertext-shaped: `n` bytes of `fill`, as base64url.
const sealedOf = (fill: number, n = 120) => base64Url(new Uint8Array(n).fill(fill))
const CRED = base64Url(new Uint8Array(16).fill(7))

const save = (user: string | null, body: unknown = { credential: CRED, uv: false, sealed: sealedOf(1) }) =>
  call('POST', user, typeof body === 'string' ? body : JSON.stringify(body))
const list = (user: string | null) => call('GET', user)
const remove = (user: string | null, body: unknown) => call('POST', user, JSON.stringify(body), '/v1/web/vault/delete')

let logged: string[] = []

beforeEach(async () => {
  await env.DB.batch([env.DB.prepare(`DELETE FROM web_vaults`), env.DB.prepare(`DELETE FROM accounts`)])
  vi.stubGlobal('fetch', async (input: any) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    return new Response('{}')
  })
  logged = []
  for (const level of ['log', 'info', 'warn', 'error', 'debug'] as const) {
    vi.spyOn(console, level).mockImplementation((...args: unknown[]) => {
      logged.push(args.map(a => (typeof a === 'string' ? a : JSON.stringify(a))).join(' '))
    })
  }
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('a browser’s sealed copy', () => {
  it('is kept for its own account and read back by it', async () => {
    expect((await save('alice')).status).toBe(200)
    const read = await list('alice')
    expect(read.status).toBe(200)
    expect(read.body).toEqual({ vaults: [{ credential: CRED, uv: false, sealed: sealedOf(1) }] })
    expect(read.headers.get('cache-control')).toBe('no-store')
  })

  it('is replaced by a newer save for the same passkey, not added to', async () => {
    await save('alice')
    await save('alice', { credential: CRED, uv: true, sealed: sealedOf(2) })
    expect((await list('alice')).body).toEqual({ vaults: [{ credential: CRED, uv: true, sealed: sealedOf(2) }] })
  })

  it('is never shown to another account, and another account can’t overwrite it', async () => {
    await save('alice')
    expect((await list('mallory')).body).toEqual({ vaults: [] })
    // Mallory's save under the same credential id lands in Mallory's own row.
    await save('mallory', { credential: CRED, uv: false, sealed: sealedOf(9) })
    expect((await list('alice')).body).toEqual({ vaults: [{ credential: CRED, uv: false, sealed: sealedOf(1) }] })
  })

  it('needs a session', async () => {
    expect((await save(null)).status).toBe(401)
    expect((await list(null)).status).toBe(401)
  })

  it('refuses what isn’t a sealed copy, and stores nothing', async () => {
    const refused = [
      { credential: CRED, uv: false, sealed: '' },
      { credential: CRED, uv: false, sealed: sealedOf(1, 20) },
      { credential: CRED, uv: false, sealed: 'not base64url!' },
      // A copy in the clear: a host key's fingerprint, or the runner list.
      { credential: CRED, uv: false, sealed: base64Url(`{"runners":[{"host_key":"SHA256:abc"}]}`.padEnd(80, ' ')) },
      { credential: '', uv: false, sealed: sealedOf(1) },
      { credential: 'a'.repeat(CREDENTIAL_MAX + 1), uv: false, sealed: sealedOf(1) },
      { credential: 'has space', uv: false, sealed: sealedOf(1) },
      { credential: CRED, uv: 'yes', sealed: sealedOf(1) },
    ]
    for (const body of refused) expect((await save('alice', body)).status, JSON.stringify(body)).toBe(400)
    expect((await save('alice', 'not json')).status).toBe(400)
    expect((await save('alice', { credential: CRED, uv: false, sealed: 'A'.repeat(VAULT_MAX + 1) })).status).toBe(413)
    expect((await list('alice')).body).toEqual({ vaults: [] })
  })

  it('holds at most VAULTS_MAX passkeys for one account, and refuses a new one rather than drop a copy', async () => {
    for (let i = 0; i < VAULTS_MAX; i++) {
      expect((await save('alice', { credential: base64Url(`cred-${i}`), uv: false, sealed: sealedOf(i + 1) })).status).toBe(200)
      await new Promise(resolve => setTimeout(resolve, 2))
    }
    // A passkey already kept is saved again at capacity, whichever row it is:
    // cred-3 is neither the oldest nor the newest.
    expect((await save('alice', { credential: base64Url('cred-3'), uv: false, sealed: sealedOf(42) })).status).toBe(200)
    const over = await save('alice', { credential: base64Url('one too many'), uv: false, sealed: sealedOf(99) })
    expect(over.status).toBe(409)
    expect(over.body).toEqual({ error: 'full' })
    const kept = (await list('alice')).body.vaults as { credential: string; sealed: string }[]
    expect(kept.map(v => v.credential).sort()).toEqual(Array.from({ length: VAULTS_MAX }, (_, i) => base64Url(`cred-${i}`)).sort())
    expect(kept.find(v => v.credential === base64Url('cred-3'))?.sealed).toBe(sealedOf(42))
  })

  it('deletes the signed-in account’s copy for one passkey, and nobody else’s', async () => {
    await save('alice')
    await save('alice', { credential: base64Url('other'), uv: false, sealed: sealedOf(3) })
    await save('mallory', { credential: CRED, uv: false, sealed: sealedOf(9) })
    expect((await remove('mallory', { credential: base64Url('other') })).status).toBe(200)
    const gone = await remove('alice', { credential: CRED })
    expect(gone.status).toBe(200)
    expect((await list('alice')).body).toEqual({ vaults: [{ credential: base64Url('other'), uv: false, sealed: sealedOf(3) }] })
    expect((await list('mallory')).body.vaults).toHaveLength(1)
    // Deleting what isn't there is still done: Sign Out doesn't need to know.
    expect((await remove('alice', { credential: CRED })).status).toBe(200)
    expect((await remove(null, { credential: CRED })).status).toBe(401)
    for (const bad of [{}, { credential: '' }, { credential: 'has space' }, { credential: 7 }]) expect((await remove('alice', bad)).status, JSON.stringify(bad)).toBe(400)
  })

  it('spends the per-account limit on every route, before any D1 read', async () => {
    const keys: string[] = []
    const limit = { limit: async ({ key }: { key: string }) => (keys.push(key), { success: keys.length <= 3 }) }
    const limited = { ...(env as any), CEREMONY_REPLY_LIMIT: limit }
    const ask = async (method: string, path: string, body?: string) =>
      (await worker.fetch(new Request(`https://relay.test${path}`, { method, headers: { authorization: `Bearer ${await sessionFor('alice')}` }, body }), limited as never, ctx)).status
    const body = JSON.stringify({ credential: CRED, uv: false, sealed: sealedOf(1) })
    const del = JSON.stringify({ credential: CRED })
    expect(await ask('POST', '/v1/web/vault', body)).toBe(200)
    expect(await ask('GET', '/v1/web/vault')).toBe(200)
    expect(await ask('POST', '/v1/web/vault/delete', del)).toBe(200)
    expect(await ask('POST', '/v1/web/vault', body)).toBe(429)
    expect(await ask('GET', '/v1/web/vault')).toBe(429)
    expect(await ask('POST', '/v1/web/vault/delete', del)).toBe(429)
    expect(keys).toEqual(Array(6).fill('alice'))
  })

  it('answers other methods 405', async () => {
    expect((await call('PUT', 'alice', '{}')).status).toBe(405)
    expect((await call('GET', 'alice', undefined, '/v1/web/vault/delete')).status).toBe(405)
  })

  it('logs nothing it was sent', async () => {
    await save('alice')
    await list('alice')
    expect(logged.join('\n')).not.toContain(sealedOf(1))
  })
})
