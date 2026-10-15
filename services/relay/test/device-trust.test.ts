import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { nameOf, withoutUnshown } from '../src/directory/common'
import { CERTIFY_NAMESPACE, certifyStatement, REGISTER_NAMESPACE, registerStatement } from '../src/directory/devices'
import { shownLabel } from '../src/directory/notices'
import { parsePublicKey } from '../src/sshsig'
import labels from '../../../test/fixtures/device-labels.json'
import { makeKey, type TestKey } from './sshsig-signer'

// Starting over with no confirmed device (ov-561, ruling R-75), and device
// names that can't masquerade (ov-560). Against the real migration chain in
// workerd's D1, as the other directory tests are.

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

/// Every push sent: APNs's alert, by token.
let pushes: { token: string; alert: { title: string; body: string } }[] = []

beforeEach(() => {
  pushes = []
  vi.stubGlobal('fetch', async (input: any, init: any = {}) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    if (url.includes('push.apple.com')) pushes.push({ token: url.split('/').pop(), alert: JSON.parse(init.body).aps.alert })
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

async function fingerprint(key: TestKey): Promise<string> {
  return (await parsePublicKey(key.line))!.fingerprint
}

/// A device signing in: a fresh challenge, signed, and its label.
async function signIn(user: string, key: TestKey, label = 'Phone', nodeKey: string | null = null): Promise<{ device: string; state: string }> {
  const challenge = (await as(user, 'devices/challenge')).body.challenge
  const statement = registerStatement(user, challenge, await fingerprint(key), null, nodeKey)
  const answer = await as(user, 'devices/register', {
    label,
    challenge,
    keyA: key.line,
    proofA: await key.sign(REGISTER_NAMESPACE, statement),
    ...(nodeKey ? { nodeKey } : {}),
  })
  expect(answer.status).toBe(200)
  return answer.body
}

async function certify(user: string, voucher: TestKey, device: TestKey, nodeKey: string | null = null) {
  const certificate = await voucher.sign(CERTIFY_NAMESPACE, certifyStatement(user, await fingerprint(device), null, nodeKey))
  expect((await as(user, 'devices/certify', { keyA: device.line, certificate })).status).toBe(200)
}

async function remove(user: string, device: string) {
  expect((await as(user, 'devices/revoke', { device })).body.ok).toBe(true)
}

async function install(user: string, token: string) {
  await as(user, 'me')
  await env.DB.prepare(
    `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at) VALUES (?, ?, 'apns', ?, 'Phone', ?)`,
  )
    .bind(crypto.randomUUID(), user, token, Date.now())
    .run()
}

async function rows(user: string): Promise<any[]> {
  return (await as(user, 'me')).body.devices
}

const trustLines = async (subject: string) =>
  (await env.DB.prepare(`SELECT COUNT(*) AS n FROM audit WHERE action = 'device.trusted' AND subject = ?`).bind(subject).first<{ n: number }>())!.n

describe('an account with no confirmed device left (R-75)', () => {
  it('trusts its next device on first use, and only once none is confirmed', async () => {
    const [mac, phone, tablet, laptop, watch] = await Promise.all([makeKey(), makeKey(), makeKey(), makeKey(), makeKey()])
    const macId = (await signIn('bob', mac, 'Mac')).device
    const phoneId = (await signIn('bob', phone, 'Phone')).device
    await certify('bob', mac, phone)
    await signIn('bob', tablet, 'Tablet')
    await certify('bob', phone, tablet)

    // The phone goes, and the tablet it certified with it (R-52); the Mac is
    // still trusted, so a new laptop waits, and so does the tablet.
    await remove('bob', phoneId)
    expect((await signIn('bob', laptop, 'Laptop')).state).toBe('pending')
    expect((await signIn('bob', tablet, 'Tablet')).state).toBe('pending')

    // The Mac goes too: nothing is confirmed, so the next device to register
    // is trusted, as a first device is.
    await remove('bob', macId)
    const watchAnswer = await signIn('bob', watch, 'Watch')
    expect(watchAnswer.state).toBe('trusted')
    expect(await trustLines(watchAnswer.device)).toBe(1)
    // And only that one: the rest wait to be certified from it.
    expect((await signIn('bob', laptop, 'Laptop')).state).toBe('pending')
    expect((await signIn('bob', tablet, 'Tablet')).state).toBe('pending')
  })

  it('trusts a pending device of theirs that signs in again, and says when', async () => {
    await install('bob', 'bobs-iphone')
    const [impostor, phone] = await Promise.all([makeKey(), makeKey()])
    const impostorId = (await signIn('bob', impostor, 'Impostor')).device
    const phoneId = (await signIn('bob', phone, 'Bob’s iPhone')).device
    await remove('bob', impostorId)
    pushes = []

    const before = Date.now()
    expect(await signIn('bob', phone, 'Bob’s iPhone')).toEqual({ device: phoneId, state: 'trusted' })
    const row = (await rows('bob')).find(d => d.id === phoneId)
    expect(row.state).toBe('trusted')
    // The banner's record: later than the day it was added, which is what
    // tells it apart from a first device's.
    expect(row.trustedAt).toBeGreaterThanOrEqual(before)
    expect(row.trustedAt).toBeGreaterThan(row.createdAt)
    expect(await trustLines(phoneId)).toBe(1)
    // The push is a hint, as it is for an added device.
    expect(pushes.map(p => p.alert.title)).toEqual(['A device is now confirmed on your account'])
    expect(pushes[0].alert.body).toContain('“Bob’s iPhone”')

    // Signing in again changes nothing and says nothing.
    pushes = []
    expect((await signIn('bob', phone, 'Bob’s iPhone')).state).toBe('trusted')
    expect(pushes).toEqual([])
    expect(await trustLines(phoneId)).toBe(1)
  })

  it("dates a first device's trust to its arrival, and keeps it once confirmed elsewhere", async () => {
    const [mac, phone] = await Promise.all([makeKey(), makeKey()])
    await signIn('carol', mac, 'Mac')
    await signIn('carol', phone, 'Phone')
    const [first, second] = await rows('carol')
    expect(first.trustedAt).toBe(first.createdAt)
    expect(second.trustedAt).toBeNull()
    // A first device is no reset: no line says so.
    expect(await trustLines(first.id)).toBe(0)
  })

  it('trusts exactly one of two pending devices signing in at once', async () => {
    const [impostor, phone, laptop] = await Promise.all([makeKey(), makeKey(), makeKey()])
    const impostorId = (await signIn('bob', impostor)).device
    await signIn('bob', phone)
    await signIn('bob', laptop)
    await remove('bob', impostorId)

    const answers = await Promise.all([signIn('bob', phone), signIn('bob', laptop)])
    expect(answers.map(a => a.state).sort()).toEqual(['pending', 'trusted'])
    const states = (await rows('bob')).map(d => d.state).sort()
    expect(states).toEqual(['pending', 'revoked', 'trusted'])
  })

  it("leaves another account's devices alone", async () => {
    const [bobs, carols, carolsPhone] = await Promise.all([makeKey(), makeKey(), makeKey()])
    const bobsId = (await signIn('bob', bobs)).device
    await signIn('carol', carols)
    await remove('bob', bobsId)
    // Bob has none confirmed, Carol has: her new phone still waits.
    expect((await signIn('carol', carolsPhone)).state).toBe('pending')
  })
})

describe('a reset, as each app is told of it (fix round 1)', () => {
  /// M1: a new device trusted by the reset is no "first device".
  it('marks a new device trusted by the reset as a reset, and pushes it as one', async () => {
    await install('bob', 'bobs-iphone')
    const [first, laptop] = await Promise.all([makeKey(), makeKey()])
    const firstId = (await signIn('bob', first, 'Mac')).device
    expect(pushes.map(p => p.alert.title)).toEqual(['A device was added to your account'])
    await remove('bob', firstId)
    pushes = []
    const laptopId = (await signIn('bob', laptop, 'Laptop')).device
    const [mac, row] = await rows('bob')
    expect(mac.trustReset).toBe(false)
    expect(row).toMatchObject({ id: laptopId, state: 'trusted', trustReset: true })
    expect(row.trustedAt).toBe(row.createdAt)
    expect(pushes.map(p => p.alert.title)).toEqual(['A device is now confirmed on your account'])
    expect(await trustLines(laptopId)).toBe(1)
  })

  it('marks a pending device promoted by the reset as a reset too', async () => {
    const [impostor, phone] = await Promise.all([makeKey(), makeKey()])
    const impostorId = (await signIn('bob', impostor)).device
    const phoneId = (await signIn('bob', phone)).device
    expect((await rows('bob')).find(d => d.id === phoneId).trustReset).toBe(false)
    await remove('bob', impostorId)
    await signIn('bob', phone)
    expect((await rows('bob')).find(d => d.id === phoneId).trustReset).toBe(true)
  })

  /// Low 1: a certified device whose node key changes is demoted, and isn't
  /// trusted again by the same request, even when it was the only one left.
  it("doesn't re-trust a certified device in the request that demotes it", async () => {
    const [mac, phone] = await Promise.all([makeKey(), makeKey()])
    const macId = (await signIn('bob', mac)).device
    const phoneId = (await signIn('bob', phone, 'Phone', `nodekey:${'aa'.repeat(32)}`)).device
    await certify('bob', mac, phone, `nodekey:${'aa'.repeat(32)}`)
    // The Mac leaves without its cascade (as a node change could leave it),
    // so the phone is the only confirmed device.
    await env.DB.prepare(`UPDATE device_keys SET state = 'pending' WHERE id = ?`).bind(macId).run()
    expect(await signIn('bob', phone, 'Phone', `nodekey:${'bb'.repeat(32)}`)).toEqual({ device: phoneId, state: 'pending' })
    expect(await trustLines(phoneId)).toBe(0)
  })

  /// Low 3: a new device and a pending one starting over at once.
  it('trusts exactly one of a new device and a pending one signing in at once', async () => {
    const [impostor, phone, laptop] = await Promise.all([makeKey(), makeKey(), makeKey()])
    const impostorId = (await signIn('bob', impostor)).device
    await signIn('bob', phone)
    await remove('bob', impostorId)
    const answers = await Promise.all([signIn('bob', phone), signIn('bob', laptop)])
    expect(answers.map(a => a.state).sort()).toEqual(['pending', 'trusted'])
    expect((await rows('bob')).filter(d => d.state === 'trusted').length).toBe(1)
  })

  /// Low 3: "none confirmed" is this account's, never anyone's.
  it("starts over though another account's device is confirmed", async () => {
    const [carols, bobs, phone, laptop] = await Promise.all([makeKey(), makeKey(), makeKey(), makeKey()])
    await signIn('carol', carols)
    const bobsId = (await signIn('bob', bobs)).device
    await signIn('bob', phone)
    await remove('bob', bobsId)
    expect((await signIn('bob', phone)).state).toBe('trusted')
    await remove('bob', (await rows('bob')).find(d => d.state === 'trusted').id)
    expect((await signIn('bob', laptop)).state).toBe('trusted')
  })
})

describe('a device label (ov-560)', () => {
  it('keeps the name it first registered with (R2)', async () => {
    await install('bob', 'bobs-iphone')
    const [mac, impostor] = await Promise.all([makeKey(), makeKey()])
    await signIn('bob', mac, 'Mac')
    const id = (await signIn('bob', impostor, 'Old Laptop')).device
    pushes = []
    await signIn('bob', impostor, 'Bob’s iPad')
    expect((await rows('bob')).find(d => d.id === id).label).toBe('Old Laptop')
    expect(pushes).toEqual([])
  })

  /// One table for every platform (`test/fixtures/device-labels.json`):
  /// AgentKit's and Android's tests read it too (R1).
  it.each(labels.cases)('is shown without what could masquerade: $why', (c: { label: string; shown: string; refused?: boolean }) => {
    expect(shownLabel(c.label)).toBe(c.shown)
    // Stored without it too: what `nameOf` keeps shows the same, and what it
    // refuses is never stored.
    const stored = nameOf(c.label)
    if (c.refused) {
      expect(stored).toBeNull()
    } else {
      expect(stored === null ? 'a device' : shownLabel(stored)).toBe(c.shown)
      if (stored !== null) expect(withoutUnshown(stored)).toBe(stored)
    }
  })

  it('is stored without bidi controls or zero-width characters', async () => {
    const id = (await signIn('bob', await makeKey(), 'iPad‮enod si​ ti')).device
    expect((await rows('bob')).find(d => d.id === id).label).toBe('iPadenod si ti')
  })
})
