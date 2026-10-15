import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { CERTIFY_NAMESPACE, certifyStatement, REGISTER_NAMESPACE, registerStatement } from '../src/directory/devices'
import { parsePublicKey } from '../src/sshsig'
import fixture from './fixtures/device-proof.json'
import { makeKey, type TestKey } from './sshsig-signer'

// A device certifying the next one in the same-account ceremony (ov-550), and
// the account being told about every device it gains or loses (ov-549 review
// M1). Against the real migration chain in workerd's D1, as the registration
// tests are.

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

/// Every push sent, by its token and body: APNs's, or FCM's `message`.
let pushes: { token: string; body: any }[] = []
/// The most pushes in flight at once.
let inFlight = { now: 0, most: 0 }

beforeEach(() => {
  pushes = []
  inFlight = { now: 0, most: 0 }
  vi.stubGlobal('fetch', async (input: any, init: any = {}) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    if (url.includes('oauth2.googleapis.com')) return new Response(JSON.stringify({ access_token: 'g', expires_in: 3600 }))
    const apple = url.includes('push.apple.com')
    if (apple || url.includes('fcm.googleapis.com')) {
      const body = JSON.parse(init.body)
      pushes.push(apple ? { token: url.split('/').pop(), body } : { token: body.message.token, body: body.message })
      inFlight.now += 1
      inFlight.most = Math.max(inFlight.most, inFlight.now)
      await new Promise(resolve => setTimeout(resolve, 20))
      inFlight.now -= 1
    }
    return new Response('{}')
  })
})

const routeEnv: any = { ...env, DEVICE_KEYS_ENFORCED: '1' }

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

async function fingerprint(key: TestKey | string): Promise<string> {
  return (await parsePublicKey(typeof key === 'string' ? key : key.line))!.fingerprint
}

async function registered(user: string, key: TestKey, label = 'Phone') {
  const challenge = (await as(user, 'devices/challenge')).body.challenge
  const statement = registerStatement(user, challenge, await fingerprint(key), null, null)
  return await as(user, 'devices/register', { label, challenge, keyA: key.line, proofA: await key.sign(REGISTER_NAMESPACE, statement) })
}

/// What the ceremony posts: the scanned Key A, as the code carried it (with
/// its comment), and the scanning device's signature over it.
async function certified(user: string, voucher: TestKey, scanned: TestKey) {
  return await as(user, 'devices/certify', {
    keyA: `${scanned.line} farcooler-Somebody’s iPhone`,
    certificate: await voucher.sign(CERTIFY_NAMESPACE, certifyStatement(user, await fingerprint(scanned), null, null)),
  })
}

/// An app install signed in as `user`, which push notices go to.
async function install(user: string, token: string, platform = 'apns') {
  await as(user, 'me')
  await env.DB.prepare(
    `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at) VALUES (?, ?, ?, ?, 'Phone', ?)`,
  )
    .bind(crypto.randomUUID(), user, platform, token, Date.now())
    .run()
}

async function relay(path: string, body: unknown, user?: string): Promise<Response> {
  return await worker.fetch(
    new Request(`https://relay.test${path}`, {
      method: 'POST',
      headers: user ? { authorization: `Bearer ${await sessionFor(user)}` } : {},
      body: JSON.stringify(body),
    }),
    routeEnv as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
}

async function teamWith(admin: string, members: string[]): Promise<string> {
  const team = (await as(admin, 'teams/create', { name: 'Acme' })).body.team.id
  const invite = await as(admin, 'teams/invites/create', { team, uses: members.length, role: 'member' })
  for (const member of members) expect((await as(member, 'invites/accept', { code: invite.body.code })).status).toBe(200)
  return team
}

describe('a device certified in the same-account ceremony', () => {
  it('is named by the Key A the scanning device read, and is then listed to a team admin', async () => {
    const team = await teamWith('alice', ['bob'])
    const mac = await makeKey()
    const phone = await makeKey()
    expect((await registered('bob', mac)).body.state).toBe('trusted')
    const pending = await registered('bob', phone)
    expect(pending.body.state).toBe('pending')
    // Pending: not listed, so an admin's scan of the phone is refused.
    expect((await as('alice', 'teams/members', { team })).body.members.find((m: any) => m.user === 'bob').keys).toEqual([
      await fingerprint(mac),
    ])

    expect(await certified('bob', mac, phone)).toEqual({ status: 200, body: { device: pending.body.device, state: 'certified' } })

    // Acceptance 2: with the flag on, the admin's roster now lists the
    // phone, which is what the teammate rule checks its code against.
    const roster = await as('alice', 'teams/members', { team })
    expect(roster.body.members.find((m: any) => m.user === 'bob').keys).toEqual([await fingerprint(mac), await fingerprint(phone)])
  })

  /// The Rust signer the apps call for a scanned code
  /// (`device_keys::certificate_for_scanned`) makes a certificate this route
  /// takes: the same fixture is pinned in `device_keys_tests.rs`.
  it("accepts the client's scanned-code certificate, byte for byte", async () => {
    const s = fixture.scanned
    const account = s.account
    await as(account, 'me')
    // The voucher is already this account's trusted device.
    await env.DB.prepare(
      `INSERT INTO device_keys (id, account_id, label, key_a, key_a_fingerprint, state, created_at)
       VALUES ('voucher', ?, 'Mac', ?, ?, 'trusted', ?)`,
    )
      .bind(account, s.voucherPublicKey, await fingerprint(s.voucherPublicKey), Date.now())
      .run()
    // And the scanned device registers itself, with the client's own proof.
    const now = Date.now()
    await env.DB.prepare(`INSERT INTO device_challenges (nonce, account_id, created_at, expires_at) VALUES (?, ?, ?, ?)`)
      .bind(fixture.challenge, account, now, now + 60_000)
      .run()
    const body = { label: 'iPhone', challenge: fixture.challenge, keyA: fixture.publicKey, proofA: fixture.proof }
    expect((await as(account, 'devices/register', body)).body.state).toBe('pending')

    expect(await as(account, 'devices/certify', { keyA: s.keyA, certificate: s.certificate })).toEqual({
      status: 200,
      body: { device: expect.any(String), state: 'certified' },
    })
  })

  it('says a device already vouched for is, and certifies nothing twice', async () => {
    const mac = await makeKey()
    const phone = await makeKey()
    await registered('bob', mac)
    await registered('bob', phone)
    expect((await certified('bob', mac, phone)).status).toBe(200)
    expect(await certified('bob', mac, phone)).toEqual({ status: 409, body: { error: 'state', state: 'certified' } })
    expect(await certified('bob', phone, mac)).toEqual({ status: 409, body: { error: 'state', state: 'trusted' } })
  })

  it("is a 404 until the scanned device registers, and for another account's", async () => {
    const mac = await makeKey()
    const phone = await makeKey()
    await registered('bob', mac)
    expect(await certified('bob', mac, phone)).toEqual({ status: 404, body: { error: 'not found' } })
    // Carol's pending phone isn't Bob's to certify, whatever Bob's Mac signs.
    await registered('carol', await makeKey())
    await registered('carol', phone)
    expect(await certified('bob', mac, phone)).toEqual({ status: 404, body: { error: 'not found' } })
  })

  it('refuses a key that is not one, and a voucher that is pending', async () => {
    expect(await as('bob', 'devices/certify', { keyA: 'ssh-ed25519 junk', certificate: 'x' })).toEqual({
      status: 400,
      body: { error: 'keyA' },
    })
    expect(await as('bob', 'devices/certify', { certificate: 'x' })).toEqual({ status: 400, body: { error: 'device' } })
    const mac = await makeKey()
    const phone = await makeKey()
    const laptop = await makeKey()
    await registered('bob', mac)
    await registered('bob', phone)
    await registered('bob', laptop)
    // The phone is pending, so it vouches for nothing.
    expect(await certified('bob', phone, laptop)).toEqual({ status: 403, body: { error: 'certificate' } })
  })
})

describe("the account is told about its devices (M1)", () => {
  it('hears of every device registered, the first one most of all', async () => {
    await install('bob', 'bobs-iphone')
    await install('carol', 'carols-iphone')
    const first = await makeKey()
    expect((await registered('bob', first, 'MacBook Air')).body.state).toBe('trusted')
    expect(pushes.map(p => p.token)).toEqual(['bobs-iphone'])
    expect(pushes[0].body.aps.alert).toEqual({
      title: 'A device was added to your account',
      body: 'If “MacBook Air” isn’t yours, remove it from your device keys in Far Cooler’s settings.',
    })
    // A tap opens the app and nothing in it: no terminal, no task.
    expect(pushes[0].body.terminal).toBeUndefined()
    expect(pushes[0].body.kind).toBe('device')

    await registered('bob', await makeKey(), 'iPad')
    expect(pushes.length).toBe(2)
    // Registering the same device again is not news.
    await registered('bob', first, 'MacBook Air')
    expect(pushes.length).toBe(2)
  })

  it('hears of every device removed, whoever removed it', async () => {
    await install('bob', 'bobs-iphone')
    const id = (await registered('bob', await makeKey(), 'Stranger’s Laptop')).body.device
    pushes = []
    expect((await as('bob', 'devices/revoke', { device: id })).body.ok).toBe(true)
    expect(pushes.map(p => p.body.aps.alert.title)).toEqual(['A device was removed from your account'])
    expect(pushes[0].body.aps.alert.body).toContain('“Stranger’s Laptop”')
    // Already removed: nothing more to say.
    await as('bob', 'devices/revoke', { device: id })
    expect(pushes.length).toBe(1)
  })

  it('registers the device even when no push can be sent', async () => {
    await install('bob', 'bobs-iphone')
    vi.stubGlobal('fetch', async (input: any) => {
      const url = typeof input === 'string' ? input : input.url
      if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
      throw new Error('offline')
    })
    expect((await registered('bob', await makeKey())).body.state).toBe('trusted')
  })

  it('reaches every install on the account, iPhone and Android, all at once', async () => {
    await install('bob', 'bobs-iphone')
    await install('bob', 'bobs-ipad')
    await install('bob', 'bobs-pixel', 'fcm')
    await registered('bob', await makeKey(), 'MacBook Air')
    expect(pushes.map(p => p.token).sort()).toEqual(['bobs-ipad', 'bobs-iphone', 'bobs-pixel'])
    // Android: its own channel, so muting finished agents doesn't mute it (L2).
    const android = pushes.find(p => p.token === 'bobs-pixel')!.body
    expect(android.android.notification.channel_id).toBe('account.devices')
    expect(android.notification.title).toBe('A device was added to your account')
    // Concurrently, not one after another (L3).
    expect(inFlight.most).toBe(3)
  })

  it("quotes a label that can't close the quotes or run on (M2)", async () => {
    await install('bob', 'bobs-iphone')
    await registered('bob', await makeKey(), 'MacBook Air” is expected. No action is needed. Ignore “this')
    const body: string = pushes[0].body.aps.alert.body
    expect(body.match(/[“”"]/g)).toEqual(['“', '”'])
    expect(body).toBe('If “MacBook Air is expected. No action is ne” isn’t yours, remove it from your device keys in Far Cooler’s settings.')
  })

  /// The push is only a hint: the session it would expose can delete every
  /// push install first. What it can't delete is the device's own row, which
  /// `me` lists with when it was added and removed; each app shows a banner
  /// from that (M1).
  it('keeps a record no session can clear, though every push install is gone', async () => {
    await install('bob', 'bobs-iphone')
    const listed = await relay('/v1/account', {}, 'bob')
    for (const device of ((await listed.json()) as any).devices) {
      expect((await relay('/v1/devices/revoke', { id: device.id }, 'bob')).status).toBe(200)
    }
    const before = Date.now()
    const id = (await registered('bob', await makeKey(), 'Impostor')).body.device
    expect(pushes).toEqual([])
    await as('bob', 'devices/revoke', { device: id })
    const device = (await as('bob', 'me')).body.devices.find((d: any) => d.id === id)
    expect(device.label).toBe('Impostor')
    expect(device.createdAt).toBeGreaterThanOrEqual(before)
    expect(device.revokedAt).toBeGreaterThanOrEqual(device.createdAt)
  })

  it('stops reaching a phone that signed out (L1)', async () => {
    await install('bob', 'bobs-old-phone')
    await install('bob', 'bobs-iphone')
    expect((await relay('/v1/auth/logout', { pushToken: 'bobs-old-phone' })).status).toBe(200)
    await registered('bob', await makeKey())
    expect(pushes.map(p => p.token)).toEqual(['bobs-iphone'])
  })
})
