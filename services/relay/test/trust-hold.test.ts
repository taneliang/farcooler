import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { CERTIFY_NAMESPACE, certifyStatement, REGISTER_NAMESPACE, registerStatement } from '../src/directory/devices'
import { holds, TRUST_RESET_HOLD_MS } from '../src/directory/hold'
import { parsePublicKey } from '../src/sshsig'
import { makeKey, type TestKey } from './sshsig-signer'

// A device trusted by a reset waits out a hold before a team's admins are
// sent its key (ov-565, ruling R-76). Against the real migration chain in
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

/// Every push sent: APNs's alert.
let pushes: { alert: { title: string; body: string } }[] = []

beforeEach(() => {
  pushes = []
  vi.stubGlobal('fetch', async (input: any, init: any = {}) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    if (url.includes('push.apple.com')) pushes.push({ alert: JSON.parse(init.body).aps.alert })
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

async function signIn(user: string, key: TestKey, label = 'Phone'): Promise<{ device: string; state: string }> {
  const challenge = (await as(user, 'devices/challenge')).body.challenge
  const statement = registerStatement(user, challenge, await fingerprint(key), null, null)
  const answer = await as(user, 'devices/register', {
    label,
    challenge,
    keyA: key.line,
    proofA: await key.sign(REGISTER_NAMESPACE, statement),
  })
  expect(answer.status).toBe(200)
  return answer.body
}

async function certify(user: string, voucher: TestKey, device: TestKey) {
  const certificate = await voucher.sign(CERTIFY_NAMESPACE, certifyStatement(user, await fingerprint(device), null, null))
  expect((await as(user, 'devices/certify', { keyA: device.line, certificate })).status).toBe(200)
}

async function teamWith(admin: string, members: string[], name = 'Acme'): Promise<string> {
  const team = (await as(admin, 'teams/create', { name })).body.team.id
  const invite = await as(admin, 'teams/invites/create', { team, uses: members.length, role: 'member' })
  for (const member of members) expect((await as(member, 'invites/accept', { code: invite.body.code })).status).toBe(200)
  return team
}

async function roster(admin: string, team: string, user: string): Promise<{ keys: string[]; held: { key: string; until: number }[] }> {
  const { keys, held } = (await as(admin, 'teams/members', { team })).body.members.find((m: any) => m.user === user)
  return { keys, held }
}

/// Bob's first device, removed, so his next one is trusted by R-75's reset.
async function resetTo(user: string, key: TestKey, label = 'Phone'): Promise<string> {
  const first = await makeKey()
  const firstId = (await signIn(user, first, 'Old Mac')).device
  expect((await as(user, 'devices/revoke', { device: firstId })).body.ok).toBe(true)
  const answer = await signIn(user, key, label)
  expect(answer.state).toBe('trusted')
  return answer.device
}

/// Move a device's reset back in time, as if `ago` milliseconds had passed.
async function trustedAgo(device: string, ago: number) {
  await env.DB.prepare(`UPDATE device_keys SET trusted_at = ? WHERE id = ?`).bind(Date.now() - ago, device).run()
}

describe('a device trusted by a reset (R-76)', () => {
  it('is held out of keys, and listed in held until the hold ends', async () => {
    const team = await teamWith('alice', ['bob'])
    const phone = await makeKey()
    const before = Date.now()
    const phoneId = await resetTo('bob', phone)

    const held = await roster('alice', team, 'bob')
    expect(held.keys).toEqual([])
    expect(held.held.map(h => h.key)).toEqual([await fingerprint(phone)])
    expect(held.held[0].until).toBeGreaterThanOrEqual(before + TRUST_RESET_HOLD_MS)
    expect(held.held[0].until).toBeLessThanOrEqual(Date.now() + TRUST_RESET_HOLD_MS)

    // A minute short of the hold, still held; past it, listed.
    await trustedAgo(phoneId, TRUST_RESET_HOLD_MS - 60_000)
    expect((await roster('alice', team, 'bob')).keys).toEqual([])
    await trustedAgo(phoneId, TRUST_RESET_HOLD_MS + 1)
    expect(await roster('alice', team, 'bob')).toEqual({ keys: [await fingerprint(phone)], held: [] })
  })

  it('holds 72 hours', () => {
    expect(TRUST_RESET_HOLD_MS).toBe(72 * 60 * 60 * 1000)
  })

  it('leaves an account’s first device alone: only a reset is held', async () => {
    const team = await teamWith('alice', ['bob'])
    const mac = await makeKey()
    await signIn('bob', mac, 'Mac')
    expect(await roster('alice', team, 'bob')).toEqual({ keys: [await fingerprint(mac)], held: [] })
  })

  it('holds a device the held one certifies, until the same moment', async () => {
    const team = await teamWith('alice', ['bob'])
    const [phone, laptop] = await Promise.all([makeKey(), makeKey()])
    const phoneId = await resetTo('bob', phone)
    expect((await signIn('bob', laptop, 'Laptop')).state).toBe('pending')
    await certify('bob', phone, laptop)

    const held = await roster('alice', team, 'bob')
    expect(held.keys).toEqual([])
    expect(held.held.map(h => h.key)).toEqual([await fingerprint(phone), await fingerprint(laptop)])
    expect(held.held[1].until).toBe(held.held[0].until)

    await trustedAgo(phoneId, TRUST_RESET_HOLD_MS + 1)
    expect((await roster('alice', team, 'bob')).keys).toEqual([await fingerprint(phone), await fingerprint(laptop)])
  })

  it('says when in me, for the held device and what it certified', async () => {
    const [phone, laptop, other] = await Promise.all([makeKey(), makeKey(), makeKey()])
    await resetTo('bob', phone)
    await signIn('bob', laptop, 'Laptop')
    await certify('bob', phone, laptop)
    await signIn('bob', other, 'Pending')
    const rows = (await as('bob', 'me')).body.devices
    const byLabel = (label: string) => rows.find((d: any) => d.label === label)
    expect(byLabel('Phone').heldUntil).toBe(byLabel('Phone').trustedAt + TRUST_RESET_HOLD_MS)
    expect(byLabel('Laptop').heldUntil).toBe(byLabel('Phone').heldUntil)
    expect(byLabel('Pending').heldUntil).toBeNull()
    expect(byLabel('Old Mac').heldUntil).toBeNull()
  })
})

describe('a broken chain (fix round 1, H1)', () => {
  it('lists nothing a demoted voucher certified: reset R certifies V, V certifies D, V changes its node key', async () => {
    const team = await teamWith('alice', ['bob'])
    const [r, v, d] = await Promise.all([makeKey(), makeKey(), makeKey()])
    const rId = await resetTo('bob', r, 'R')
    await signIn('bob', v, 'V')
    await certify('bob', r, v)
    await signIn('bob', d, 'D')
    await certify('bob', v, d)
    // The hold ends, so R and its chain would be listed.
    await trustedAgo(rId, TRUST_RESET_HOLD_MS + 1)
    expect((await roster('alice', team, 'bob')).keys).toEqual([await fingerprint(r), await fingerprint(v), await fingerprint(d)])

    // V signs in with a new node key: back to pending, and D with it.
    const node = `nodekey:${'ab'.repeat(32)}`
    const challenge = (await as('bob', 'devices/challenge')).body.challenge
    const statement = registerStatement('bob', challenge, await fingerprint(v), null, node)
    const again = await as('bob', 'devices/register', {
      label: 'V', challenge, keyA: v.line, proofA: await v.sign(REGISTER_NAMESPACE, statement), nodeKey: node,
    })
    expect(again.body.state).toBe('pending')
    const states = Object.fromEntries((await as('bob', 'me')).body.devices.map((x: any) => [x.label, x.state]))
    expect(states).toMatchObject({ R: 'trusted', V: 'pending', D: 'pending' })
    expect(await roster('alice', team, 'bob')).toEqual({ keys: [await fingerprint(r)], held: [] })
  })

  it('lists nothing whose voucher was demoted behind the cascade’s back', async () => {
    const team = await teamWith('alice', ['bob'])
    const [r, v, d] = await Promise.all([makeKey(), makeKey(), makeKey()])
    await resetTo('bob', r, 'R')
    const vId = (await signIn('bob', v, 'V')).device
    await certify('bob', r, v)
    await signIn('bob', d, 'D')
    await certify('bob', v, d)
    // Whatever path leaves D certified under a voucher that isn't, D's chain
    // reaches no trusted device: it's in neither list.
    await env.DB.prepare(`UPDATE device_keys SET state = 'pending' WHERE id = ?`).bind(vId).run()
    expect(await roster('alice', team, 'bob')).toEqual({ keys: [], held: [{ key: await fingerprint(r), until: expect.any(Number) }] })
    expect((await as('alice', 'teams/devices/confirm', { team, user: 'bob', keyA: d.line })).status).toBe(409)
  })
})

describe('a team admin confirming a held device in person', () => {
  it('is announced to the person, by push and in me (fix round 1, M1)', async () => {
    const team = await teamWith('alice', ['bob'])
    await env.DB.prepare(
      `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at) VALUES (?, 'bob', 'apns', 'bobs-phone', 'Phone', ?)`,
    )
      .bind(crypto.randomUUID(), Date.now())
      .run()
    const phone = await makeKey()
    await resetTo('bob', phone, 'Bob’s Phone')
    pushes = []
    expect((await as('alice', 'teams/devices/confirm', { team, user: 'bob', keyA: phone.line })).status).toBe(200)
    expect(pushes.map(p => p.alert)).toEqual([
      {
        title: 'A team admin confirmed one of your devices',
        body: 'An admin of “Acme” confirmed “Bob’s Phone”, so that team’s runners can add it now. If it isn’t yours, remove it from your device keys in Far Cooler’s settings.',
      },
    ])
    const row = (await as('bob', 'me')).body.devices.find((d: any) => d.label === 'Bob’s Phone')
    expect(row.holdConfirmations).toEqual([{ team: 'Acme', at: expect.any(Number) }])
    // Bob deleting his push installs doesn't clear the record.
    await env.DB.prepare(`DELETE FROM devices WHERE account_id = 'bob'`).run()
    expect((await as('bob', 'me')).body.devices.find((d: any) => d.label === 'Bob’s Phone').holdConfirmations).toHaveLength(1)
  })

  it('lists it to that team at once, with an audit line', async () => {
    const team = await teamWith('alice', ['bob'])
    const phone = await makeKey()
    const phoneId = await resetTo('bob', phone)
    expect(await as('alice', 'teams/devices/confirm', { team, user: 'bob', keyA: phone.line })).toEqual({
      status: 200,
      body: { ok: true },
    })
    expect(await roster('alice', team, 'bob')).toEqual({ keys: [await fingerprint(phone)], held: [] })
    const line = await env.DB.prepare(`SELECT actor, subject, detail FROM audit WHERE team_id = ? AND action = 'device.hold_confirmed'`)
      .bind(team)
      .first<{ actor: string; subject: string; detail: string }>()
    expect(line?.actor).toBe('alice')
    expect(line?.subject).toBe(phoneId)
    expect(JSON.parse(line!.detail).keyA).toBe(await fingerprint(phone))
  })

  it('vouches for that team only', async () => {
    const acme = await teamWith('alice', ['bob'])
    const other = await teamWith('carol', ['bob'], 'Other')
    const phone = await makeKey()
    await resetTo('bob', phone)
    expect((await as('alice', 'teams/devices/confirm', { team: acme, user: 'bob', keyA: phone.line })).status).toBe(200)
    expect((await roster('alice', acme, 'bob')).keys).toEqual([await fingerprint(phone)])
    expect((await roster('carol', other, 'bob')).keys).toEqual([])
    // And Alice can't confirm for a team she isn't an admin of.
    expect((await as('alice', 'teams/devices/confirm', { team: other, user: 'bob', keyA: phone.line })).status).toBe(404)
  })

  it('never counts for a reset that came after it', async () => {
    const team = await teamWith('alice', ['bob'])
    const phone = await makeKey()
    const phoneId = await resetTo('bob', phone)
    expect((await as('alice', 'teams/devices/confirm', { team, user: 'bob', keyA: phone.line })).status).toBe(200)
    // Trusted again by a later reset, after the confirmation.
    await env.DB.prepare(`UPDATE device_keys SET trusted_at = ? WHERE id = ?`).bind(Date.now() + 1000, phoneId).run()
    expect((await roster('alice', team, 'bob')).keys).toEqual([])
  })

  it('refuses an admin’s own device, a non-admin, and a member outside the team', async () => {
    const team = await teamWith('alice', ['bob', 'dave'])
    const [phone, alices] = await Promise.all([makeKey(), makeKey()])
    await resetTo('bob', phone)
    await resetTo('alice', alices)
    // A stolen admin session can't lift its own hold.
    expect((await as('alice', 'teams/devices/confirm', { team, user: 'alice', keyA: alices.line })).body).toEqual({
      error: 'own device',
    })
    expect((await roster('alice', team, 'alice')).keys).toEqual([])
    // Dave is a member, not an admin.
    expect((await as('dave', 'teams/devices/confirm', { team, user: 'bob', keyA: phone.line })).status).toBe(403)
    // Erin isn't in the team at all, so her device is nobody's to confirm here.
    const erins = await makeKey()
    await resetTo('erin', erins)
    expect((await as('alice', 'teams/devices/confirm', { team, user: 'erin', keyA: erins.line })).status).toBe(404)
    // Nor is a key that belongs to someone else in the team.
    expect((await as('alice', 'teams/devices/confirm', { team, user: 'dave', keyA: phone.line })).status).toBe(404)
    expect((await roster('alice', team, 'bob')).keys).toEqual([])
  })

  it('writes nothing for a device that isn’t held', async () => {
    const team = await teamWith('alice', ['bob'])
    const mac = await makeKey()
    await signIn('bob', mac, 'Mac')
    expect((await as('alice', 'teams/devices/confirm', { team, user: 'bob', keyA: mac.line })).body).toEqual({ ok: true })
    const rows = await env.DB.prepare(`SELECT COUNT(*) AS n FROM device_hold_confirmations`).first<{ n: number }>()
    expect(rows!.n).toBe(0)
  })
})

describe('holds', () => {
  const now = 1_000_000_000_000
  const row = (id: string, state: string, extra: Partial<{ trust_reset: number; trusted_at: number; certified_by: string }> = {}) => ({
    id,
    state,
    trust_reset: extra.trust_reset ?? null,
    trusted_at: extra.trusted_at ?? null,
    certified_by: extra.certified_by ?? null,
  })

  it('ends a cycle, and lists nothing on it (fix round 1, H1)', () => {
    const rows = [row('a', 'certified', { certified_by: 'b' }), row('b', 'certified', { certified_by: 'a' })]
    const standing = holds(rows, now)
    expect(standing.since.size).toBe(0)
    expect([...standing.unrooted]).toEqual(['a', 'b'])
  })

  it('fails closed on a voucher that is missing, pending or revoked (fix round 1, H1)', () => {
    const rows = [
      row('pending', 'pending'),
      row('revoked', 'revoked'),
      row('a', 'certified', { certified_by: 'pending' }),
      row('b', 'certified', { certified_by: 'revoked' }),
      row('c', 'certified', { certified_by: 'gone' }),
      row('d', 'certified'),
      row('e', 'certified', { certified_by: 'a' }),
    ]
    const standing = holds(rows, now)
    expect([...standing.unrooted].sort()).toEqual(['a', 'b', 'c', 'd', 'e'])
    expect(standing.since.size).toBe(0)
  })

  it('follows a chain only through certified devices to a reset', () => {
    const rows = [
      row('root', 'trusted', { trust_reset: 1, trusted_at: now - 1000 }),
      row('child', 'certified', { certified_by: 'root' }),
      row('grandchild', 'certified', { certified_by: 'child' }),
      row('first', 'trusted', { trusted_at: now - 1000 }),
      row('beside', 'certified', { certified_by: 'first' }),
    ]
    expect(holds(rows, now).unrooted.size).toBe(0)
    expect([...holds(rows, now).since]).toEqual([
      ['root', now - 1000],
      ['child', now - 1000],
      ['grandchild', now - 1000],
    ])
  })
})
