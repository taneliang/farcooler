import { env } from 'cloudflare:test'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { REPLY_MAX, REPLY_TTL_MS } from '../src/ceremony-reply'
import fixture from './fixtures/ceremony-reply.json'

// The sealed ceremony reply (ov-769): who may post one, who may read it, that
// a read takes it, that it doesn't outlive 120 s, and that nothing in the
// clear reaches D1 or the logs. Against the real migration chain in workerd's
// D1, because the reader check and the delete-on-read are both SQL.
//
// `fixtures/ceremony-reply.json` is a reply sealed by the Rust code itself
// (`write_the_relays_fixture` in crates/ceremony/src/seal_tests.rs), and
// `the_relays_fixture_is_a_real_sealed_reply` opens it, so the host key it
// carries is really inside the ciphertext this suite posts.

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

function fromBase64Url(text: string): string {
  const padded = text.replaceAll('-', '+').replaceAll('_', '/')
  return atob(padded.padEnd(Math.ceil(padded.length / 4) * 4, '='))
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

async function call(method: string, path: string, user: string | null, body?: string): Promise<{ status: number; body: any; headers: Headers }> {
  const response = await worker.fetch(
    new Request(`https://relay.test${path}`, {
      method,
      headers: user ? { authorization: `Bearer ${await sessionFor(user)}` } : {},
      body,
    }),
    env as never,
    ctx,
  )
  return { status: response.status, body: await response.json(), headers: response.headers }
}

const CEREMONY = fixture.offer.ceremony
const ALICE = fixture.offer.account

const post = (user: string | null, body: unknown = { ceremony: CEREMONY, account: ALICE, sealed: fixture.sealed }) =>
  call('POST', '/v1/ceremony/reply', user, typeof body === 'string' ? body : JSON.stringify(body))
const take = (user: string | null, ceremony = CEREMONY) => call('GET', `/v1/ceremony/reply/${ceremony}`, user)

async function stored(): Promise<Record<string, unknown>[]> {
  return (await env.DB.prepare(`SELECT * FROM ceremony_replies`).all()).results
}

/// `admin` an admin and `member` a member of one team; `role` sets the first's.
async function team(admin: string, member: string, role: 'admin' | 'member' = 'admin'): Promise<void> {
  const now = Date.now()
  await env.DB.batch([
    env.DB.prepare(`INSERT OR IGNORE INTO accounts (id, created_at) VALUES (?, ?), (?, ?)`).bind(admin, now, member, now),
    env.DB.prepare(`INSERT INTO teams (id, name, created_at) VALUES ('team_1', 'Team', ?)`).bind(now),
    env.DB.prepare(`INSERT INTO memberships (team_id, account_id, role, joined_at) VALUES ('team_1', ?, ?, ?), ('team_1', ?, 'member', ?)`).bind(
      admin,
      role,
      now,
      member,
      now,
    ),
  ])
}

let logged: string[] = []

beforeEach(async () => {
  await env.DB.batch([
    env.DB.prepare(`DELETE FROM ceremony_replies`),
    env.DB.prepare(`DELETE FROM memberships`),
    env.DB.prepare(`DELETE FROM teams`),
    env.DB.prepare(`DELETE FROM accounts`),
  ])
  vi.stubGlobal('fetch', async (input: any) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    return new Response('{}')
  })
  logged = []
  for (const level of ['log', 'info', 'warn', 'error', 'debug'] as const) {
    vi.spyOn(console, level).mockImplementation((...args: unknown[]) => {
      logged.push(args.map(a => (a instanceof Error ? `${a.message} ${a.stack}` : typeof a === 'string' ? a : JSON.stringify(a))).join(' '))
    })
  }
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('the sealed ceremony reply', () => {
  it('goes from the offer’s own account to it, once', async () => {
    expect((await post(ALICE)).status).toBe(200)

    const first = await take(ALICE)
    expect(first.status).toBe(200)
    expect(first.body).toEqual({ sealed: fixture.sealed })
    expect(first.headers.get('cache-control')).toBe('no-store')

    // A read deletes it: the second read gets nothing.
    expect((await take(ALICE)).status).toBe(404)
    expect(await stored()).toEqual([])
  })

  it('refuses a reply posted by an unrelated account, and stores nothing', async () => {
    const answer = await post('mallory')
    expect(answer.status).toBe(403)
    expect(await stored()).toEqual([])
  })

  it('takes a reply from an admin of a team the account is in', async () => {
    await team('carol', ALICE)
    expect((await post('carol')).status).toBe(200)
    expect((await take(ALICE)).body).toEqual({ sealed: fixture.sealed })
  })

  it('refuses one from a team member who isn’t its admin', async () => {
    await team('dave', ALICE, 'member')
    expect((await post('dave')).status).toBe(403)
  })

  it('refuses one from an admin of a team the account isn’t in', async () => {
    await team('carol', 'someone_else')
    expect((await post('carol')).status).toBe(403)
  })

  it('lets only the offer’s account read it, and another’s read takes nothing', async () => {
    await post(ALICE)
    expect((await take('mallory')).status).toBe(404)
    // The poster too, when the poster is a teammate's admin.
    await team('carol', ALICE)
    expect((await take('carol')).status).toBe(404)
    expect((await take(ALICE)).status).toBe(200)
  })

  it('is gone 120 s after it was posted', async () => {
    const start = Date.now()
    await post(ALICE)
    vi.spyOn(Date, 'now').mockReturnValue(start + REPLY_TTL_MS + 1000)
    expect((await take(ALICE)).status).toBe(404)
  })

  it('is still there just inside the window', async () => {
    const start = Date.now()
    await post(ALICE)
    vi.spyOn(Date, 'now').mockReturnValue(start + REPLY_TTL_MS - 1000)
    expect((await take(ALICE)).status).toBe(200)
  })

  it('refuses a body over 4 KB, and takes one at exactly 4 KB', async () => {
    expect(REPLY_MAX).toBe(4096)
    const body = JSON.stringify({ ceremony: CEREMONY, account: ALICE, sealed: fixture.sealed })
    const atLimit = body + ' '.repeat(REPLY_MAX - body.length)
    expect((await post(ALICE, atLimit + ' ')).status).toBe(413)
    expect(await stored()).toEqual([])
    expect((await post(ALICE, atLimit)).status).toBe(200)
  })

  it('keeps the first reply to a ceremony and refuses a second', async () => {
    await team('carol', ALICE)
    expect((await post(ALICE)).status).toBe(200)
    const other = { ceremony: CEREMONY, account: ALICE, sealed: base64Url(new Uint8Array(64).fill(7)) }
    expect((await post('carol', other)).status).toBe(409)
    expect((await take(ALICE)).body).toEqual({ sealed: fixture.sealed })
  })

  it('takes a new reply to a ceremony whose old one outlived its window', async () => {
    const start = Date.now()
    await post(ALICE)
    vi.spyOn(Date, 'now').mockReturnValue(start + REPLY_TTL_MS + 1000)
    const other = { ceremony: CEREMONY, account: ALICE, sealed: base64Url(new Uint8Array(64).fill(7)) }
    expect((await post(ALICE, other)).status).toBe(200)
    expect((await take(ALICE)).body).toEqual({ sealed: other.sealed })
  })

  it('needs a session on both routes', async () => {
    expect((await post(null)).status).toBe(401)
    expect((await take(null)).status).toBe(401)
  })

  it('takes each route by one method only', async () => {
    expect((await call('GET', '/v1/ceremony/reply', ALICE)).status).toBe(405)
    expect((await call('POST', `/v1/ceremony/reply/${CEREMONY}`, ALICE, '{}')).status).toBe(405)
    // And everything else is still POST only.
    expect((await call('GET', '/v1/account', ALICE)).status).toBe(405)
  })

  it('refuses a malformed ceremony, account or sealed reply', async () => {
    const good = { ceremony: CEREMONY, account: ALICE, sealed: fixture.sealed }
    for (const bad of [
      { ...good, ceremony: 'XYZ' },
      { ...good, ceremony: CEREMONY.toUpperCase() },
      { ...good, account: '' },
      { ...good, sealed: '' },
      { ...good, sealed: 'not base64url!' },
      { ...good, sealed: base64Url(new Uint8Array(48)) },
    ]) {
      expect((await post(ALICE, bad)).status, JSON.stringify(bad)).toBe(400)
    }
    expect((await post(ALICE, 'not json')).status).toBe(400)
    expect((await take(ALICE, 'not-a-ceremony')).status).toBe(404)
    expect(await stored()).toEqual([])
  })

  it('is swept by the five-minute cron once its window is over', async () => {
    const start = Date.now()
    await post(ALICE)
    await env.DB.prepare(`INSERT INTO ceremony_replies (ceremony, account_id, posted_by, sealed, created_at) VALUES (?, ?, ?, 'x', ?)`)
      .bind('f'.repeat(32), ALICE, ALICE, start + 60_000)
      .run()
    vi.spyOn(Date, 'now').mockReturnValue(start + REPLY_TTL_MS + 1000)
    await worker.scheduled({} as ScheduledController, env as never)
    expect((await stored()).map(row => row.ceremony)).toEqual(['f'.repeat(32)])
  })
})

describe('what the relay holds of a reply', () => {
  const HOST_KEY = fixture.host_key

  /// Every place a reply could have landed in the clear: each column of each
  /// row, the sealed column decoded, and every line logged.
  async function everywhere(): Promise<string> {
    const rows = await stored()
    const decoded = rows.map(row => (typeof row.sealed === 'string' ? fromBase64Url(row.sealed) : ''))
    return [JSON.stringify(rows), ...decoded, ...logged].join('\n')
  }

  it('is ciphertext: the row and the logs never hold the reply’s host key', async () => {
    // Inside the ciphertext, as the Rust test proves, and nowhere in the clear.
    expect(fixture.manifest).toContain(HOST_KEY)
    expect(fixture.sealed).not.toContain(HOST_KEY)

    expect((await post(ALICE)).status).toBe(200)
    expect(await stored()).toHaveLength(1)
    const held = await everywhere()
    expect(held).toContain(fixture.sealed)
    expect(held).not.toContain(HOST_KEY)
    expect(held).not.toContain('box.tail-1234')
    // And the logs hold nothing of a reply at all, not even its ciphertext.
    expect(logged.join('\n')).not.toContain(fixture.sealed.slice(0, 40))
  })

  it('refuses a reply posted in the clear, and keeps none of it', async () => {
    for (const sealed of [base64Url(fixture.manifest), base64Url(`{"runners":[]}${'x'.repeat(64)}`)]) {
      expect((await post(ALICE, { ceremony: CEREMONY, account: ALICE, sealed })).status).toBe(400)
      // Base64 hides the host key from a plain search, so look for the body itself.
      expect(logged.join('\n')).not.toContain(sealed.slice(0, 40))
    }
    expect(await stored()).toEqual([])
    expect(await everywhere()).not.toContain(HOST_KEY)
  })
})
