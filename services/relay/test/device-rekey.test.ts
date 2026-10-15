import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { CERTIFY_NAMESPACE, certifyStatement, REGISTER_NAMESPACE, registerStatement } from '../src/directory/devices'
import { parsePublicKey } from '../src/sshsig'
import { makeKey, type TestKey } from './sshsig-signer'

// A phone makes a new key (ov-569): the new key registers, is confirmed, and
// is listed again; and when the old key confirmed it, revoking the old key
// hands the old key's place to the new one instead of unconfirming the
// person's other devices. Against the real migration chain in workerd's D1.

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

async function registered(user: string, key: TestKey, label = 'iPhone') {
  const challenge = (await as(user, 'devices/challenge')).body.challenge
  const statement = registerStatement(user, challenge, await fingerprint(key), null, null)
  return await as(user, 'devices/register', { label, challenge, keyA: key.line, proofA: await key.sign(REGISTER_NAMESPACE, statement) })
}

async function certified(user: string, voucher: TestKey, scanned: TestKey) {
  return await as(user, 'devices/certify', {
    keyA: scanned.line,
    certificate: await voucher.sign(CERTIFY_NAMESPACE, certifyStatement(user, await fingerprint(scanned), null, null)),
  })
}

async function rows(user: string): Promise<Record<string, { state: string; certifiedBy: string | null }>> {
  const me = await as(user, 'me')
  return Object.fromEntries(me.body.devices.map((d: any) => [d.id, { state: d.state, certifiedBy: d.certifiedBy ?? null }]))
}

async function me(user: string, id: string): Promise<any> {
  return (await as(user, 'me')).body.devices.find((d: any) => d.id === id)
}

async function teamWith(admin: string, members: string[]): Promise<string> {
  const team = (await as(admin, 'teams/create', { name: 'Acme' })).body.team.id
  const invite = await as(admin, 'teams/invites/create', { team, uses: members.length, role: 'member' })
  for (const member of members) expect((await as(member, 'invites/accept', { code: invite.body.code })).status).toBe(200)
  return team
}

async function listed(admin: string, team: string, user: string): Promise<string[]> {
  return (await as(admin, 'teams/members', { team })).body.members.find((m: any) => m.user === user).keys
}

describe('a phone whose key was revoked', () => {
  // The card's acceptance: a stolen session removed the phone, so its Key A
  // can never come back. A new key registers, is confirmed from the person's
  // Mac, and is listed to their team's admin again.
  it('makes a new key, registers it, and is confirmed and listed again', async () => {
    const team = await teamWith('alice', ['bob'])
    const mac = await makeKey()
    const old = await makeKey()
    await registered('bob', mac, 'Mac')
    const phone = await registered('bob', old)
    expect((await certified('bob', mac, old)).status).toBe(200)
    expect((await as('bob', 'devices/revoke', { device: phone.body.device })).body.ok).toBe(true)
    // The old key is gone for good.
    expect((await registered('bob', old)).body).toEqual({ error: 'revoked' })
    expect(await listed('alice', team, 'bob')).toEqual([await fingerprint(mac)])

    const fresh = await makeKey()
    const again = await registered('bob', fresh)
    expect(again.body.state).toBe('pending')
    expect(await listed('alice', team, 'bob')).toEqual([await fingerprint(mac)])
    expect((await certified('bob', mac, fresh)).body).toEqual({ device: again.body.device, state: 'certified' })
    expect(await listed('alice', team, 'bob')).toEqual([await fingerprint(mac), await fingerprint(fresh)])
  })
})

describe('a phone replacing a key that is still confirmed', () => {
  /// Bob's phone was his first device, and it confirmed his Mac. The phone
  /// makes a new key and confirms it itself, with the old key.
  async function phoneThatConfirmedTheMac() {
    const old = await makeKey()
    const mac = await makeKey()
    const laptop = await makeKey()
    const phone = (await registered('bob', old)).body.device
    const macId = (await registered('bob', mac, 'Mac')).body.device
    const laptopId = (await registered('bob', laptop, 'Laptop')).body.device
    expect((await certified('bob', old, mac)).status).toBe(200)
    // The Mac confirmed the laptop: two links below the phone.
    expect((await certified('bob', mac, laptop)).status).toBe(200)
    const fresh = await makeKey()
    const next = (await registered('bob', fresh)).body.device
    expect((await certified('bob', old, fresh)).body.state).toBe('certified')
    return { phone, macId, laptopId, next, fresh, mac, laptopKey: laptop }
  }

  it('hands the old key its place in the chain, so nothing else is unconfirmed', async () => {
    const team = await teamWith('alice', ['bob'])
    const { phone, macId, laptopId, next, fresh, mac, laptopKey } = await phoneThatConfirmedTheMac()
    const before = await me('bob', phone)
    const revoked = await as('bob', 'devices/revoke', { device: phone, successor: next })
    expect(revoked.body).toEqual({ ok: true, uncertified: 0, moved: 1 })
    const after = await rows('bob')
    expect(after[phone].state).toBe('revoked')
    // The phone was the account's first, trusted device: its new key is now.
    expect(after[next]).toEqual({ state: 'trusted', certifiedBy: null })
    expect((await me('bob', next)).trustedAt).toBe(before.trustedAt)
    expect(after[macId]).toEqual({ state: 'certified', certifiedBy: next })
    expect(after[laptopId]).toEqual({ state: 'certified', certifiedBy: macId })
    // Every chain still reaches a trusted device, so the team's admin is sent
    // all three keys, the new one included, and not the old one.
    const listed = await as('alice', 'teams/members', { team })
    const keys = listed.body.members.find((m: any) => m.user === 'bob').keys
    expect(new Set(keys)).toEqual(new Set([await fingerprint(fresh), await fingerprint(mac), await fingerprint(laptopKey)]))
    const audit = await env.DB.prepare(`SELECT action, subject FROM audit WHERE action = 'device.replaced'`).all()
    expect(audit.results).toEqual([{ action: 'device.replaced', subject: phone }])

    // And the chain still holds: removing the new key now unconfirms the Mac,
    // as removing the old one would have.
    expect((await as('bob', 'devices/revoke', { device: next })).body.uncertified).toBe(2)
    expect((await rows('bob'))[macId].state).toBe('pending')
  })

  it('a certified old key hands the new one its own voucher', async () => {
    const team = await teamWith('alice', ['bob'])
    const mac = await makeKey()
    const old = await makeKey()
    const macId = (await registered('bob', mac, 'Mac')).body.device
    const phone = (await registered('bob', old)).body.device
    expect((await certified('bob', mac, old)).status).toBe(200)
    const fresh = await makeKey()
    const next = (await registered('bob', fresh)).body.device
    expect((await certified('bob', old, fresh)).status).toBe(200)
    expect((await as('bob', 'devices/revoke', { device: phone, successor: next })).body.ok).toBe(true)
    expect((await rows('bob'))[next]).toEqual({ state: 'certified', certifiedBy: macId })
    const keys = (await as('alice', 'teams/members', { team })).body.members.find((m: any) => m.user === 'bob').keys
    expect(new Set(keys)).toEqual(new Set([await fingerprint(mac), await fingerprint(fresh)]))
  })

  it("keeps a reset's hold running to the same end, so a new key is no way around it", async () => {
    const team = await teamWith('alice', ['bob'])
    const first = await makeKey()
    const firstId = (await registered('bob', first, 'Mac')).body.device
    const old = await makeKey()
    const phone = (await registered('bob', old)).body.device
    // Bob removes his only confirmed device, so the phone is trusted by R-75's
    // reset when it signs in again, and held.
    await as('bob', 'devices/revoke', { device: firstId })
    expect((await registered('bob', old)).body.state).toBe('trusted')
    const held = (await me('bob', phone)).heldUntil
    expect(held).toBeGreaterThan(Date.now())

    const fresh = await makeKey()
    const next = (await registered('bob', fresh)).body.device
    expect((await certified('bob', old, fresh)).status).toBe(200)
    expect((await as('bob', 'devices/revoke', { device: phone, successor: next })).body.ok).toBe(true)
    expect((await me('bob', next)).heldUntil).toBe(held)
    const bob = (await as('alice', 'teams/members', { team })).body.members.find((m: any) => m.user === 'bob')
    expect(bob.keys).toEqual([])
  })

  it('without a successor, revoking the old key unconfirms everything it confirmed (R-52)', async () => {
    const { phone, macId, laptopId, next } = await phoneThatConfirmedTheMac()
    expect((await as('bob', 'devices/revoke', { device: phone })).body.uncertified).toBe(3)
    const after = await rows('bob')
    expect([after[macId].state, after[laptopId].state, after[next].state]).toEqual(['pending', 'pending', 'pending'])
  })

  it('refuses a successor the old key did not confirm, and revokes nothing', async () => {
    const { phone, macId, laptopId } = await phoneThatConfirmedTheMac()
    // A device a stolen session registered: pending, so nobody's successor.
    const stranger = (await registered('bob', await makeKey(), 'iPhone')).body.device
    expect(await as('bob', 'devices/revoke', { device: phone, successor: stranger })).toEqual({
      status: 409,
      body: { error: 'successor' },
    })
    // The laptop is certified, but by the Mac, not by the phone.
    expect((await as('bob', 'devices/revoke', { device: phone, successor: laptopId })).status).toBe(409)
    // Another account's device, whoever confirmed it.
    const carols = (await registered('carol', await makeKey())).body.device
    expect((await as('bob', 'devices/revoke', { device: phone, successor: carols })).status).toBe(409)
    expect((await as('bob', 'devices/revoke', { device: phone, successor: phone })).status).toBe(400)
    const after = await rows('bob')
    expect(after[phone].state).toBe('trusted')
    expect(after[macId].state).toBe('certified')
  })
})
