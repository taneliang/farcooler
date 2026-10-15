import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import {
  CERTIFY_NAMESPACE,
  CHALLENGES_PER_ACCOUNT,
  certifyStatement,
  REGISTER_NAMESPACE,
  registerStatement,
} from '../src/directory/devices'
import { parsePublicKey, verifySignature } from '../src/sshsig'
import wranglerToml from '../wrangler.toml?raw'
import fixture from './fixtures/device-proof.json'
import { makeKey, type TestKey } from './sshsig-signer'

// A device registering its Key A over a relay-chosen challenge, and a team's
// admins reading those keys (ov-549). Against the real migration chain in
// workerd's D1: the single use of a challenge and the admin-only read are SQL,
// and a fake of either would agree with whatever the code believed.

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

async function sessionFor(user: string): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const claims = { sub: user, iss: (env as any).WORKOS_ISSUER, client_id: (env as any).WORKOS_CLIENT_ID, sid: 's', iat: now, exp: now + 3600 }
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const payload = base64Url(JSON.stringify(claims))
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', signing.privateKey, new TextEncoder().encode(`${header}.${payload}`))
  return `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
}

beforeEach(() => {
  vi.stubGlobal('fetch', async (input: any) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    return new Response('{}')
  })
})

/// The keys in `teams/members` are behind `DEVICE_KEYS_ENFORCED` (R-72), off
/// by default; these tests run with it on, but for the one that says so.
let routeEnv: any = { ...env, DEVICE_KEYS_ENFORCED: '1' }

async function as(user: string, route: string, body: unknown = {}): Promise<{ status: number; body: any }> {
  const response = await worker.fetch(
    new Request(`https://relay.test/v1/directory/${route}`, {
      method: 'POST',
      headers: { authorization: `Bearer ${await sessionFor(user)}` },
      body: JSON.stringify(body),
    }),
    routeEnv as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
  return { status: response.status, body: await response.json() }
}

async function fingerprint(key: TestKey): Promise<string> {
  return (await parsePublicKey(key.line))!.fingerprint
}

async function challengeFor(user: string): Promise<string> {
  const minted = await as(user, 'devices/challenge')
  expect(minted.status).toBe(200)
  return minted.body.challenge
}

/// The body an app sends for `key`, as `user`, over `challenge`.
async function proven(user: string, key: TestKey, challenge: string) {
  const statement = registerStatement(user, challenge, await fingerprint(key), null, null)
  return { label: 'Phone', challenge, keyA: key.line, proofA: await key.sign(REGISTER_NAMESPACE, statement) }
}

async function registered(user: string, key: TestKey) {
  return await as(user, 'devices/register', await proven(user, key, await challengeFor(user)))
}

async function devices(): Promise<number> {
  return (await env.DB.prepare(`SELECT COUNT(*) AS n FROM device_keys`).first<{ n: number }>())!.n
}

describe('a device registering its Key A', () => {
  it('is proven by a signature over a challenge the relay chose', async () => {
    const minted = await as('alice', 'devices/challenge')
    expect(minted.body).toEqual({ challenge: expect.stringMatching(/^[0-9a-f]{64}$/), expiresAt: expect.any(Number) })
    expect((await as('alice', 'devices/challenge')).body.challenge).not.toBe(minted.body.challenge)

    const phone = await makeKey()
    const answer = await as('alice', 'devices/register', await proven('alice', phone, minted.body.challenge))
    expect(answer.body).toEqual({ device: expect.any(String), state: 'trusted' })
  })

  /// The Rust signer the apps call (`crates/client/src/device_keys.rs`) signs
  /// these exact bytes, and its tests pin the same file.
  it("accepts the client's own signer, byte for byte", async () => {
    const account = fixture.account
    expect(registerStatement(account, fixture.challenge, (await parsePublicKey(fixture.publicKey))!.fingerprint, null, null)).toBe(fixture.statement)
    const now = Date.now()
    await as(account, 'me')
    await env.DB.prepare(`INSERT INTO device_challenges (nonce, account_id, created_at, expires_at) VALUES (?, ?, ?, ?)`)
      .bind(fixture.challenge, account, now, now + 60_000)
      .run()
    const body = { label: 'Mac', challenge: fixture.challenge, keyA: fixture.publicKey, proofA: fixture.proof }
    expect((await as(account, 'devices/register', body)).body).toEqual({ device: expect.any(String), state: 'trusted' })
  })

  it("accepts the client's certificate signer, byte for byte", async () => {
    const c = fixture.certificate
    const keyA = (await parsePublicKey(c.keyA))!.fingerprint
    expect(certifyStatement(c.account, keyA, null, c.node)).toBe(c.statement)
    const signer = await verifySignature(c.certificate, CERTIFY_NAMESPACE, c.statement)
    expect(signer?.fingerprint).toBe((await parsePublicKey(c.voucherPublicKey))!.fingerprint)
  })

  it('refuses a proof used twice', async () => {
    const phone = await makeKey()
    const body = await proven('alice', phone, await challengeFor('alice'))
    expect((await as('alice', 'devices/register', body)).status).toBe(200)
    expect(await as('alice', 'devices/register', body)).toEqual({ status: 403, body: { error: 'challenge' } })
    // And a device revoked since can't come back with it, nor with anything.
    const id = (await as('alice', 'me')).body.devices[0].id
    await as('alice', 'devices/revoke', { device: id })
    expect((await as('alice', 'devices/register', body)).body).toEqual({ error: 'challenge' })
  })

  it('lets exactly one of two requests carrying one challenge through', async () => {
    const challenge = await challengeFor('alice')
    const [a, b] = await Promise.all([
      as('alice', 'devices/register', await proven('alice', await makeKey(), challenge)),
      as('alice', 'devices/register', await proven('alice', await makeKey(), challenge)),
    ])
    expect([a.status, b.status].sort()).toEqual([200, 403])
    expect(await devices()).toBe(1)
  })

  it('refuses a challenge that expired, was never minted, or was minted for someone else', async () => {
    const phone = await makeKey()
    const stale = await challengeFor('alice')
    await env.DB.prepare(`UPDATE device_challenges SET expires_at = ? WHERE nonce = ?`).bind(Date.now() - 1, stale).run()
    expect((await as('alice', 'devices/register', await proven('alice', phone, stale))).body).toEqual({ error: 'challenge' })

    const invented = 'ab'.repeat(32)
    expect((await as('alice', 'devices/register', await proven('alice', phone, invented))).body).toEqual({ error: 'challenge' })

    // Bob's challenge, signed for Alice's account: the signature is good, the
    // challenge is nobody's here, and it stays Bob's.
    const bobs = await challengeFor('bob')
    expect((await as('alice', 'devices/register', await proven('alice', phone, bobs))).body).toEqual({ error: 'challenge' })
    expect((await as('bob', 'devices/register', await proven('bob', phone, bobs))).body.state).toBe('trusted')
    expect(await devices()).toBe(1)
  })

  it('refuses a challenge in any shape but the one minted', async () => {
    const phone = await makeKey()
    for (const challenge of [undefined, '', 'AB'.repeat(32), 'ab'.repeat(31), `${'ab'.repeat(32)}\nkey-a x`]) {
      const body = { ...(await proven('alice', phone, 'ab'.repeat(32))), challenge }
      expect((await as('alice', 'devices/register', body)).body).toEqual({ error: 'challenge' })
    }
    expect(await devices()).toBe(0)
  })

  it("refuses a wrong signature, and doesn't spend the challenge on it", async () => {
    const phone = await makeKey()
    const other = await makeKey()
    const challenge = await challengeFor('alice')
    const statement = registerStatement('alice', challenge, await fingerprint(phone), null, null)
    const forged = { ...(await proven('alice', phone, challenge)), proofA: await other.sign(REGISTER_NAMESPACE, statement) }
    expect(await as('alice', 'devices/register', forged)).toEqual({ status: 403, body: { error: 'proofA' } })
    const held = await env.DB.prepare(`SELECT COUNT(*) AS n FROM device_challenges WHERE nonce = ?`).bind(challenge).first<{ n: number }>()
    expect(held!.n).toBe(1)
    // Signed by the right key over another challenge.
    const elsewhere = await proven('alice', phone, 'cd'.repeat(32))
    expect((await as('alice', 'devices/register', { ...elsewhere, challenge })).body).toEqual({ error: 'proofA' })
    expect(await devices()).toBe(0)
    expect((await as('alice', 'devices/register', await proven('alice', phone, challenge))).status).toBe(200)
  })

  it('keeps only the newest challenges an account minted', async () => {
    const first = await challengeFor('alice')
    for (let i = 0; i < CHALLENGES_PER_ACCOUNT + 4; i++) await challengeFor('alice')
    const held = await env.DB.prepare(`SELECT COUNT(*) AS n FROM device_challenges WHERE account_id = 'alice'`).first<{ n: number }>()
    expect(held!.n).toBe(CHALLENGES_PER_ACCOUNT)
    expect((await as('alice', 'devices/register', await proven('alice', await makeKey(), first))).body).toEqual({ error: 'challenge' })
  })

  it('are minted only within the per-account limit, refused before any write', async () => {
    const asked: string[] = []
    let allow = false
    routeEnv = {
      ...routeEnv,
      DEVICE_CHALLENGE_LIMIT: { async limit({ key }: { key: string }) { asked.push(key); return { success: allow } } },
    }
    try {
      expect(await as('alice', 'devices/challenge')).toEqual({ status: 429, body: { error: 'slow down' } })
      const rows = await env.DB.prepare(`SELECT COUNT(*) AS n FROM device_challenges`).first<{ n: number }>()
      expect(rows!.n).toBe(0)
      // Keyed on the account, not the address: one person's loop is one budget.
      expect(asked).toEqual(['alice'])
      allow = true
      expect((await as('alice', 'devices/challenge')).status).toBe(200)
    } finally {
      routeEnv = { ...env, DEVICE_KEYS_ENFORCED: '1' }
    }
    // No binding at all is open, as a local relay has none.
    expect((env as any).DEVICE_CHALLENGE_LIMIT).toBeUndefined()
    expect((await as('alice', 'devices/challenge')).status).toBe(200)
  })

  it('are throttled on every channel that is deployed', () => {
    const blocks = wranglerToml.split(/^\[\[(?:env\.[a-z]+\.)?unsafe\.bindings\]\]$/m).slice(1)
    const limiters = blocks.filter(block => /name = "DEVICE_CHALLENGE_LIMIT"/.test(block))
    expect(limiters.length).toBe(4)
    for (const block of limiters) {
      expect(block).toContain('type = "ratelimit"')
      expect(block).toMatch(/simple = \{ limit = \d+, period = \d+ \}/)
    }
    const namespaces = [...wranglerToml.matchAll(/namespace_id = "(\d+)"/g)].map(m => m[1])
    expect(new Set(namespaces).size).toBe(namespaces.length)
  })

  it('are pruned by an index, not a scan', async () => {
    const plan = await env.DB.prepare(`EXPLAIN QUERY PLAN DELETE FROM device_challenges WHERE expires_at <= ?`).bind(0).all<{ detail: string }>()
    expect(plan.results.map(r => r.detail).join(' ')).toContain('device_challenges_by_expiry')
  })

  it('needs a session', async () => {
    const response = await worker.fetch(
      new Request('https://relay.test/v1/directory/devices/challenge', { method: 'POST', body: '{}' }),
      env as never,
      { waitUntil() {}, passThroughOnException() {} } as never,
    )
    expect(response.status).toBe(401)
  })
})

/// A team `admin` made, with `members` joined by invite as `role`.
async function teamWith(admin: string, members: string[], role = 'member'): Promise<string> {
  const team = (await as(admin, 'teams/create', { name: 'Acme' })).body.team.id
  if (members.length) {
    const invite = await as(admin, 'teams/invites/create', { team, uses: members.length, role })
    for (const member of members) expect((await as(member, 'invites/accept', { code: invite.body.code })).status).toBe(200)
  }
  return team
}

function keysOf(roster: any, user: string): unknown {
  return roster.members.find((m: any) => m.user === user)?.keys
}

describe("a team member's registered keys", () => {
  /// Alice is an admin of Acme, where Bob and Erin are members and Dan is its
  /// billing contact. Carol is an admin of another team, which Erin is also in.
  async function acme() {
    const team = await teamWith('alice', ['bob', 'erin'])
    const invite = await as('alice', 'teams/invites/create', { team, uses: 1, role: 'billing' })
    await as('dan', 'invites/accept', { code: invite.body.code })
    const other = await teamWith('carol', ['erin'])
    const phone = await makeKey()
    const laptop = await makeKey()
    const old = await makeKey()
    expect((await registered('bob', phone)).body.state).toBe('trusted')
    expect((await registered('bob', laptop)).body.state).toBe('pending')
    const oldId = (await registered('bob', old)).body.device
    // Certified, then revoked: a key that was good once and isn't now.
    await as('bob', 'devices/certify', {
      device: oldId,
      certificate: await phone.sign(CERTIFY_NAMESPACE, certifyStatement('bob', await fingerprint(old), null, null)),
    })
    await as('bob', 'devices/revoke', { device: oldId })
    const erins = await makeKey()
    await registered('erin', erins)
    return { team, other, phone, laptop, erins }
  }

  it('are read by an admin of their team: trusted and certified Key A only', async () => {
    const { team, phone, erins } = await acme()
    const roster = await as('alice', 'teams/members', { team })
    expect(roster.status).toBe(200)
    expect(keysOf(roster.body, 'bob')).toEqual([await fingerprint(phone)])
    expect(keysOf(roster.body, 'erin')).toEqual([await fingerprint(erins)])
    // A member with nothing registered is told as none, not left out.
    expect(keysOf(roster.body, 'alice')).toEqual([])
    expect(keysOf(roster.body, 'dan')).toEqual([])
  })

  it('include a pending device once another of theirs certifies it', async () => {
    const { team, phone, laptop } = await acme()
    const me = await as('bob', 'me')
    const pending = me.body.devices.find((d: any) => d.state === 'pending').id
    await as('bob', 'devices/certify', {
      device: pending,
      certificate: await phone.sign(CERTIFY_NAMESPACE, certifyStatement('bob', await fingerprint(laptop), null, null)),
    })
    const roster = await as('alice', 'teams/members', { team })
    expect(keysOf(roster.body, 'bob')).toEqual([await fingerprint(phone), await fingerprint(laptop)])
  })

  it('are sent to nobody while the relay is not set to enforce them', async () => {
    const { team } = await acme()
    routeEnv = env
    try {
      const roster = await as('alice', 'teams/members', { team })
      expect(roster.status).toBe(200)
      expect(roster.body.members.every((m: any) => !('keys' in m))).toBe(true)
      expect(JSON.stringify(roster.body)).not.toMatch(/SHA256:/)
    } finally {
      routeEnv = { ...env, DEVICE_KEYS_ENFORCED: '1' }
    }
  })

  it("are read by nobody else: not a member, not billing, not another team's admin", async () => {
    const { team, other } = await acme()
    const asMember = await as('bob', 'teams/members', { team })
    expect(asMember.status).toBe(200)
    expect(asMember.body.members.every((m: any) => !('keys' in m))).toBe(true)
    expect(JSON.stringify(asMember.body)).not.toMatch(/SHA256:/)

    expect((await as('dan', 'teams/members', { team })).status).toBe(403)
    expect(await as('carol', 'teams/members', { team })).toEqual({ status: 404, body: { error: 'not found' } })
    expect((await as('mallory', 'teams/members', { team })).status).toBe(404)

    // Carol's own team: Erin's keys, and nothing of Bob's, who isn't in it.
    const hers = await as('carol', 'teams/members', { team: other })
    expect(keysOf(hers.body, 'erin')).toHaveLength(1)
    expect(JSON.stringify(hers.body)).not.toContain('"bob"')
  })

  it("stop once the member is removed, and once the reader isn't an admin", async () => {
    const { team, phone } = await acme()
    await as('alice', 'teams/members/role', { team, user: 'erin', role: 'admin' })
    await as('erin', 'teams/members/role', { team, user: 'alice', role: 'member' })
    const demoted = await as('alice', 'teams/members', { team })
    expect(demoted.body.members.every((m: any) => !('keys' in m))).toBe(true)

    await as('erin', 'teams/members/remove', { team, user: 'bob' })
    const roster = await as('erin', 'teams/members', { team })
    expect(roster.body.members.map((m: any) => m.user)).not.toContain('bob')
    // Listed among the removed, for readmitting, with no keys.
    expect(roster.body.removed.map((r: any) => r.user)).toEqual(['bob'])
    expect(JSON.stringify(roster.body)).not.toContain(await fingerprint(phone))
  })
})
