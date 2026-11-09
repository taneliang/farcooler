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

async function call(method: string, user: string | null, body?: string): Promise<{ status: number; body: any; headers: Headers }> {
  const response = await worker.fetch(
    new Request('https://relay.test/v1/web/vault', { method, headers: user ? { authorization: `Bearer ${await sessionFor(user)}` } : {}, body }),
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

  it('holds at most VAULTS_MAX passkeys for one account', async () => {
    for (let i = 0; i < VAULTS_MAX; i++) expect((await save('alice', { credential: base64Url(`cred-${i}`), uv: false, sealed: sealedOf(i + 1) })).status).toBe(200)
    const over = await save('alice', { credential: base64Url('one too many'), uv: false, sealed: sealedOf(99) })
    expect(over.status).toBe(409)
    expect(over.body).toEqual({ error: 'full' })
    // A passkey already kept can still be saved again.
    expect((await save('alice', { credential: base64Url('cred-0'), uv: false, sealed: sealedOf(42) })).status).toBe(200)
    expect((await list('alice')).body.vaults).toHaveLength(VAULTS_MAX)
  })

  it('answers other methods 405', async () => {
    expect((await call('PUT', 'alice', '{}')).status).toBe(405)
  })

  it('logs nothing it was sent', async () => {
    await save('alice')
    await list('alice')
    expect(logged.join('\n')).not.toContain(sealedOf(1))
  })
})
