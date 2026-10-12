import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { CERTIFY_NAMESPACE, certifyStatement, REGISTER_NAMESPACE, registerStatement } from '../src/directory/devices'
import { runnerKey } from '../src/directory/runners'
import { parsePublicKey } from '../src/sshsig'
import { makeKey, type TestKey } from './sshsig-signer'

// The team directory (ov-487), against the real migration chain in workerd's
// D1. Every guard here is SQL or a signature check, and a fake of either would
// agree with whatever the code believed.

// MARK: - Sessions, signed for real

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

async function sessionFor(user: string, email = `${user}@example.test`): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const claims = {
    sub: user,
    email,
    iss: (env as any).WORKOS_ISSUER,
    client_id: (env as any).WORKOS_CLIENT_ID,
    sid: 'session_test',
    iat: now,
    exp: now + 3600,
  }
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

async function call(path: string, body: unknown, bearer?: string): Promise<{ status: number; body: any }> {
  const response = await worker.fetch(
    new Request(`https://relay.test${path}`, {
      method: 'POST',
      headers: bearer ? { authorization: `Bearer ${bearer}` } : {},
      body: JSON.stringify(body),
    }),
    env as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
  return { status: response.status, body: await response.json() }
}

/// Call a directory route as `user`.
async function as(user: string, route: string, body: unknown = {}, email?: string) {
  return await call(`/v1/directory/${route}`, body, await sessionFor(user, email))
}

async function personalTeam(user: string): Promise<string> {
  const me = await as(user, 'me')
  return me.body.teams.find((t: any) => t.personal).id
}

/// A team `admin` made, with `members` joined by invite.
async function teamWith(admin: string, members: string[] = [], role = 'member'): Promise<string> {
  const team = (await as(admin, 'teams/create', { name: 'Acme' })).body.team.id
  if (members.length) {
    const invite = await as(admin, 'teams/invites/create', { team, uses: members.length, role })
    for (const member of members) expect((await as(member, 'invites/accept', { code: invite.body.code })).status).toBe(200)
  }
  return team
}

async function auditActions(team: string): Promise<string[]> {
  const rows = await env.DB.prepare(`SELECT action FROM audit WHERE team_id = ? ORDER BY id`).bind(team).all<{ action: string }>()
  return rows.results.map(r => r.action)
}

// MARK: - Personal teams

describe('personal teams', () => {
  it('are made the first time someone reaches the directory, once, with them as admin', async () => {
    const first = await as('alice', 'me')
    expect(first.status).toBe(200)
    expect(first.body.teams).toEqual([{ id: expect.any(String), name: 'Personal', personal: true, role: 'admin' }])
    const again = await as('alice', 'me')
    expect(again.body.teams).toEqual(first.body.teams)
    const count = await env.DB.prepare(`SELECT COUNT(*) AS n FROM teams WHERE personal_of = 'alice'`).first<{ n: number }>()
    expect(count?.n).toBe(1)
  })

  it('need a session', async () => {
    expect((await call('/v1/directory/me', {})).status).toBe(401)
  })

  it("can't be left, shared by invite, or have roles changed", async () => {
    const team = await personalTeam('alice')
    expect((await as('alice', 'teams/members/remove', { team, user: 'alice' })).body).toEqual({ error: 'personal' })
    expect((await as('alice', 'teams/invites/create', { team })).body).toEqual({ error: 'personal' })
    expect((await as('alice', 'teams/members/role', { team, user: 'alice', role: 'member' })).body).toEqual({ error: 'personal' })
  })

  it('are invisible to everyone else', async () => {
    const team = await personalTeam('alice')
    expect((await as('mallory', 'teams/members', { team })).status).toBe(404)
  })
})

// MARK: - Teams and members

describe('teams', () => {
  it('make their creator the admin and log it', async () => {
    const made = await as('alice', 'teams/create', { name: '  Acme  ' })
    expect(made.body.team).toEqual({ id: expect.any(String), name: 'Acme', personal: false, role: 'admin' })
    const members = await as('alice', 'teams/members', { team: made.body.team.id })
    expect(members.body.members).toEqual([{ user: 'alice', email: 'alice@example.test', role: 'admin', joinedAt: expect.any(Number) }])
    expect(await auditActions(made.body.team.id)).toEqual(['team.created'])
  })

  it.each([[''], ['x'.repeat(81)], ['a\nb'], [42]])('refuse the name %j', async name => {
    expect((await as('alice', 'teams/create', { name })).status).toBe(400)
  })

  it('answer a stranger as if the team did not exist', async () => {
    const team = await teamWith('alice')
    for (const route of ['teams/members', 'teams/runners', 'teams/audit', 'teams/invites/list']) {
      expect((await as('mallory', route, { team })).status).toBe(404)
    }
  })

  it('show billing members nothing yet', async () => {
    const team = await teamWith('alice', ['bill'], 'billing')
    expect((await as('bill', 'teams/members', { team })).status).toBe(403)
    expect((await as('bill', 'teams/runners', { team })).status).toBe(403)
  })
})

describe('roles', () => {
  it('are changed by admins only, and logged', async () => {
    const team = await teamWith('alice', ['bob', 'carol'])
    expect((await as('bob', 'teams/members/role', { team, user: 'carol', role: 'admin' })).status).toBe(403)
    expect((await as('alice', 'teams/members/role', { team, user: 'bob', role: 'admin' })).body).toEqual({ ok: true })
    const row = await env.DB.prepare(`SELECT role FROM memberships WHERE team_id = ? AND account_id = 'bob'`).bind(team).first()
    expect(row).toEqual({ role: 'admin' })
    expect(await auditActions(team)).toContain('member.role_changed')
  })

  it('never leave a team without an admin', async () => {
    const team = await teamWith('alice', ['bob'])
    expect((await as('alice', 'teams/members/role', { team, user: 'alice', role: 'member' })).body).toEqual({ error: 'last admin' })
    expect((await as('alice', 'teams/members/remove', { team, user: 'alice' })).body).toEqual({ error: 'last admin' })
    // With a second admin, the first may step down.
    await as('alice', 'teams/members/role', { team, user: 'bob', role: 'admin' })
    expect((await as('alice', 'teams/members/role', { team, user: 'alice', role: 'member' })).body).toEqual({ ok: true })
    expect((await as('bob', 'teams/members/role', { team, user: 'bob', role: 'member' })).body).toEqual({ error: 'last admin' })
    expect(await auditActions(team)).toEqual(['team.created', 'invite.created', 'member.joined', 'member.role_changed', 'member.role_changed'])
  })

  it('refuse a role that does not exist', async () => {
    const team = await teamWith('alice', ['bob'])
    expect((await as('alice', 'teams/members/role', { team, user: 'bob', role: 'owner' })).status).toBe(400)
  })
})

describe('removing a member', () => {
  it("takes their roles on the team's runners, and only that team's", async () => {
    const team = await teamWith('alice', ['bob'])
    const work = (await as('alice', 'runners/register', { team, install: 'work-box', name: 'Build box' })).body.runner
    const home = (await as('bob', 'runners/register', { install: 'bob-laptop' })).body.runner
    await grant(work, 'bob', 'control')

    const removed = await as('alice', 'teams/members/remove', { team, user: 'bob' })
    expect(removed.body).toEqual({ ok: true, runners: 1 })
    const roles = await env.DB.prepare(`SELECT runner_id, role FROM runner_roles WHERE account_id = 'bob'`).all()
    expect(roles.results).toEqual([{ runner_id: home, role: 'owner' }])
    expect((await as('bob', 'teams/runners', { team })).status).toBe(404)

    const line = await env.DB.prepare(`SELECT detail FROM audit WHERE action = 'member.removed'`).first<{ detail: string }>()
    expect(JSON.parse(line!.detail)).toEqual({ runners: [{ runner: work, role: 'control' }] })
  })

  it('is for admins, but anyone may leave', async () => {
    const team = await teamWith('alice', ['bob', 'carol'])
    expect((await as('bob', 'teams/members/remove', { team, user: 'carol' })).status).toBe(403)
    expect((await as('bob', 'teams/members/remove', { team, user: 'bob' })).body).toEqual({ ok: true, runners: 0 })
    expect(await auditActions(team)).toContain('member.left')
  })

  it('says so for someone who is not in the team', async () => {
    const team = await teamWith('alice')
    expect((await as('alice', 'teams/members/remove', { team, user: 'nobody' })).status).toBe(404)
  })
})

// MARK: - Invites

describe('invites', () => {
  it('are made by admins, shown once, and stored only as a hash', async () => {
    const team = await teamWith('alice', ['bob'])
    expect((await as('bob', 'teams/invites/create', { team })).status).toBe(403)
    const made = await as('alice', 'teams/invites/create', { team })
    expect(made.body.code).toMatch(/^[0-9a-f]{64}$/)
    const rows = await env.DB.prepare(`SELECT * FROM invites`).all()
    expect(JSON.stringify(rows.results)).not.toContain(made.body.code)
    const listed = await as('alice', 'teams/invites/list', { team })
    expect(JSON.stringify(listed.body)).not.toContain(made.body.code)
  })

  it('join someone as the role they carry, once per person', async () => {
    const team = await teamWith('alice')
    const { code } = (await as('alice', 'teams/invites/create', { team, uses: 2 })).body
    expect((await as('bob', 'invites/accept', { code })).body).toEqual({ team: { id: team, name: 'Acme' }, role: 'member', joined: true })
    // Again: already in, and no second use spent.
    expect((await as('bob', 'invites/accept', { code })).body.joined).toBe(false)
    expect((await env.DB.prepare(`SELECT uses FROM invites`).first())).toEqual({ uses: 1 })
  })

  it('stop at their use limit', async () => {
    const team = await teamWith('alice')
    const { code } = (await as('alice', 'teams/invites/create', { team, uses: 1 })).body
    expect((await as('bob', 'invites/accept', { code })).status).toBe(200)
    expect((await as('carol', 'invites/accept', { code })).status).toBe(404)
  })

  it('stop when revoked or expired', async () => {
    const team = await teamWith('alice')
    const revoked = (await as('alice', 'teams/invites/create', { team })).body
    expect((await as('alice', 'teams/invites/revoke', { team, invite: revoked.invite })).body).toEqual({ ok: true })
    expect((await as('bob', 'invites/accept', { code: revoked.code })).status).toBe(404)

    const expired = (await as('alice', 'teams/invites/create', { team })).body
    await env.DB.prepare(`UPDATE invites SET expires_at = ? WHERE id = ?`).bind(Date.now() - 1, expired.invite).run()
    expect((await as('bob', 'invites/accept', { code: expired.code })).status).toBe(404)
  })

  it('hold to their domain', async () => {
    const team = await teamWith('alice')
    const { code } = (await as('alice', 'teams/invites/create', { team, domain: 'Acme.com', uses: 5 })).body
    expect((await as('bob', 'invites/accept', { code }, 'bob@evil.com')).status).toBe(403)
    expect((await as('eve', 'invites/accept', { code }, 'eve@notacme.com')).status).toBe(403)
    expect((await as('carol', 'invites/accept', { code }, 'carol@acme.com')).status).toBe(200)
  })

  it('never make an admin', async () => {
    const team = await teamWith('alice')
    expect((await as('alice', 'teams/invites/create', { team, role: 'admin' })).status).toBe(400)
  })

  it('refuse a code that was never issued', async () => {
    expect((await as('bob', 'invites/accept', { code: 'f'.repeat(64) })).status).toBe(404)
  })
})

// MARK: - Runners

/// What phase 5 will write from a signed grant. Phase 1 has no grant route, so
/// tests that need a non-owner role put it there directly.
async function grant(runner: string, user: string, role: string) {
  await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES (?, ?) ON CONFLICT (id) DO NOTHING`).bind(user, Date.now()).run()
  await env.DB.prepare(
    `INSERT INTO runner_roles (runner_id, account_id, role, granted_by, signature, granted_at) VALUES (?, ?, ?, 'test', 'sig', ?)`,
  )
    .bind(runner, user, role, Date.now())
    .run()
}

describe('registering a runner', () => {
  it('makes the registrant its owner, in their personal team by default', async () => {
    const made = await as('alice', 'runners/register', { install: 'install-1', name: 'Laptop' })
    expect(made.status).toBe(200)
    expect(made.body.team).toBe(await personalTeam('alice'))
    const listed = await as('alice', 'teams/runners', { team: made.body.team })
    expect(listed.body.runners).toEqual([
      {
        id: made.body.runner,
        name: 'Laptop',
        createdAt: expect.any(Number),
        lastSeenAt: null,
        version: null,
        owner: 'alice',
        unclaimed: false,
        access: [{ user: 'alice', email: 'alice@example.test', role: 'owner', grantedAt: expect.any(Number) }],
      },
    ])
    expect(await auditActions(made.body.team)).toEqual(['runner.registered'])
  })

  it('issues a token the runner beats with, like a pairing did', async () => {
    const { token } = (await as('alice', 'runners/register', { install: 'install-1', name: 'Laptop' })).body
    const beat = await call('/v1/heartbeat', { install: 'install-1', name: 'Laptop', every: 300 }, token)
    expect(beat.status).toBe(200)
    expect((await call('/v1/heartbeat', { install: 'install-1' }, 'not-a-token')).status).toBe(401)
  })

  it.each([[undefined], [''], ['has space'], ['x'.repeat(65)], [42]])('requires an install id, refusing %j', async install => {
    expect((await as('alice', 'runners/register', { install })).body).toEqual({ error: 'install' })
    expect((await env.DB.prepare(`SELECT COUNT(*) AS n FROM daemons`).first())).toEqual({ n: 0 })
  })

  it('never stores the raw install id', async () => {
    await as('alice', 'runners/register', { install: 'raw-install-id-123' })
    const everything = [
      ...(await env.DB.prepare(`SELECT * FROM runners`).all()).results,
      ...(await env.DB.prepare(`SELECT * FROM daemons`).all()).results,
      ...(await env.DB.prepare(`SELECT * FROM audit`).all()).results,
    ]
    expect(JSON.stringify(everything)).not.toContain('raw-install-id-123')
    expect((await env.DB.prepare(`SELECT install_key FROM runners`).first())).toEqual({ install_key: await runnerKey('raw-install-id-123') })
  })

  it('refuses a runner someone else already registered, naming no team', async () => {
    const team = await teamWith('alice', ['bob'])
    await as('alice', 'runners/register', { team, install: 'box' })
    expect((await as('bob', 'runners/register', { team, install: 'box' })).body).toEqual({ error: 'registered' })
    expect((await as('mallory', 'runners/register', { install: 'box' })).body).toEqual({ error: 'registered' })
  })

  it("won't move a runner to another team, even for its owner", async () => {
    await as('alice', 'runners/register', { install: 'box' })
    const team = await teamWith('alice')
    expect((await as('alice', 'runners/register', { team, install: 'box' })).body).toEqual({ error: 'registered' })
  })

  it('rotates the token when its owner registers it again, and the old one stops', async () => {
    const first = (await as('alice', 'runners/register', { install: 'box', name: 'Box' })).body
    const second = (await as('alice', 'runners/register', { install: 'box', name: 'Box 2' })).body
    expect(second.runner).toBe(first.runner)
    expect((await call('/v1/heartbeat', { install: 'box' }, first.token)).status).toBe(401)
    expect((await call('/v1/heartbeat', { install: 'box' }, second.token)).status).toBe(200)
    expect(await auditActions(first.team)).toEqual(['runner.registered', 'runner.token_rotated'])
  })

  it('is for admins and members of the team only', async () => {
    const team = await teamWith('alice', ['bill'], 'billing')
    expect((await as('bill', 'runners/register', { team, install: 'box' })).status).toBe(403)
    expect((await as('mallory', 'runners/register', { team, install: 'box' })).status).toBe(404)
  })
})

describe('taking a runner role away', () => {
  it("is for a team admin, or the runner's owner or admin", async () => {
    const team = await teamWith('alice', ['bob', 'carol', 'dave'])
    const runner = (await as('bob', 'runners/register', { team, install: 'box' })).body.runner
    await grant(runner, 'carol', 'control')
    await grant(runner, 'dave', 'read')

    // Carol is a member with control, not the runner's admin.
    expect((await as('carol', 'runners/roles/remove', { runner, user: 'dave' })).status).toBe(403)
    // Bob owns it.
    expect((await as('bob', 'runners/roles/remove', { runner, user: 'dave' })).body).toEqual({ ok: true })
    // Alice is a team admin with no role on the runner at all.
    expect((await as('alice', 'runners/roles/remove', { runner, user: 'carol' })).body).toEqual({ ok: true })
    const left = await env.DB.prepare(`SELECT account_id FROM runner_roles WHERE runner_id = ?`).bind(runner).all()
    expect(left.results).toEqual([{ account_id: 'bob' }])
    expect(await auditActions(team)).toContain('runner.role_removed')
  })

  it("can't take the owner's", async () => {
    const team = await teamWith('alice', ['bob'])
    const runner = (await as('bob', 'runners/register', { team, install: 'box' })).body.runner
    expect((await as('alice', 'runners/roles/remove', { runner, user: 'bob' })).body).toEqual({ error: 'owner' })
  })

  it('looks like no runner at all to someone outside its team', async () => {
    const runner = (await as('alice', 'runners/register', { install: 'box' })).body.runner
    expect((await as('mallory', 'runners/roles/remove', { runner, user: 'alice' })).status).toBe(404)
  })

  it('reports a runner with no owner or admin left as unclaimed', async () => {
    const team = await teamWith('alice', ['bob'])
    await as('bob', 'runners/register', { team, install: 'box' })
    await as('alice', 'teams/members/remove', { team, user: 'bob' })
    const listed = await as('alice', 'teams/runners', { team })
    expect(listed.body.runners[0]).toMatchObject({ owner: null, unclaimed: true, access: [] })
  })
})

// MARK: - Devices

interface Device {
  keyA: TestKey
  keyB?: TestKey
  node?: string
}

async function fingerprint(key: TestKey): Promise<string> {
  return (await parsePublicKey(key.line))!.fingerprint
}

/// Register `device` as `user`, proving each key, as an app would.
async function registerDevice(user: string, device: Device, overrides: Record<string, unknown> = {}) {
  const statement = registerStatement(
    user,
    await fingerprint(device.keyA),
    device.keyB ? await fingerprint(device.keyB) : null,
    device.node ?? null,
  )
  return await as(user, 'devices/register', {
    label: 'iPhone',
    keyA: device.keyA.line,
    proofA: await device.keyA.sign(REGISTER_NAMESPACE, statement),
    ...(device.keyB ? { keyB: device.keyB.line, proofB: await device.keyB.sign(REGISTER_NAMESPACE, statement) } : {}),
    ...(device.node ? { nodeKey: device.node } : {}),
    ...overrides,
  })
}

/// `voucher` signs a certificate for `device`, as one of `user`'s devices.
async function certificate(user: string, voucher: TestKey, device: Device): Promise<string> {
  return await voucher.sign(
    CERTIFY_NAMESPACE,
    certifyStatement(user, await fingerprint(device.keyA), device.keyB ? await fingerprint(device.keyB) : null, device.node ?? null),
  )
}

describe('devices', () => {
  it("trust a person's first device, and hold every later one until certified", async () => {
    const phone = { keyA: await makeKey() }
    const mac = { keyA: await makeKey(), keyB: await makeKey(), node: `nodekey:${'ab'.repeat(32)}` }
    expect((await registerDevice('alice', phone)).body).toEqual({ device: expect.any(String), state: 'trusted' })
    const pending = await registerDevice('alice', mac)
    expect(pending.body.state).toBe('pending')

    const certified = await as('alice', 'devices/certify', { device: pending.body.device, certificate: await certificate('alice', phone.keyA, mac) })
    expect(certified.body).toEqual({ device: pending.body.device, state: 'certified' })
    const me = await as('alice', 'me')
    expect(me.body.devices.map((d: any) => d.state)).toEqual(['trusted', 'certified'])
  })

  it('store public keys only, without their comments', async () => {
    const mac = { keyA: await makeKey('alice@secret-host'), keyB: await makeKey('shell') }
    await registerDevice('alice', mac)
    const row = await env.DB.prepare(`SELECT key_a, key_b FROM device_keys`).first<{ key_a: string; key_b: string }>()
    expect(row!.key_a).toBe(mac.keyA.line.replace(' alice@secret-host', ''))
    expect(row!.key_b).toBe(mac.keyB.line.replace(' shell', ''))
    expect(JSON.stringify(await env.DB.prepare(`SELECT * FROM device_keys`).all())).not.toMatch(/PRIVATE|secret-host/)
  })

  it('refuse a private key sent as a public one', async () => {
    const response = await as('alice', 'devices/register', {
      label: 'x',
      keyA: '-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----',
      proofA: '',
    })
    expect(response.body).toEqual({ error: 'keyA' })
    expect((await env.DB.prepare(`SELECT COUNT(*) AS n FROM device_keys`).first())).toEqual({ n: 0 })
  })

  it('refuse a key without proof that the device holds it', async () => {
    const phone = { keyA: await makeKey() }
    const other = await makeKey()
    const statement = registerStatement('alice', await fingerprint(phone.keyA), null, null)
    // Signed by a different key.
    expect((await registerDevice('alice', phone, { proofA: await other.sign(REGISTER_NAMESPACE, statement) })).body).toEqual({ error: 'proofA' })
    // Signed by the right key, but for someone else's profile.
    const forBob = registerStatement('bob', await fingerprint(phone.keyA), null, null)
    expect((await registerDevice('alice', phone, { proofA: await phone.keyA.sign(REGISTER_NAMESPACE, forBob) })).body).toEqual({ error: 'proofA' })
    // Signed by the right key, in another namespace.
    expect((await registerDevice('alice', phone, { proofA: await phone.keyA.sign(CERTIFY_NAMESPACE, statement) })).body).toEqual({ error: 'proofA' })
    expect((await env.DB.prepare(`SELECT COUNT(*) AS n FROM device_keys`).first())).toEqual({ n: 0 })
  })

  it("refuse a Key B the device can't prove it holds", async () => {
    const mac = { keyA: await makeKey(), keyB: await makeKey() }
    const statement = registerStatement('alice', await fingerprint(mac.keyA), await fingerprint(mac.keyB), null)
    const response = await registerDevice('alice', mac, { proofB: await mac.keyA.sign(REGISTER_NAMESPACE, statement) })
    expect(response.body).toEqual({ error: 'proofB' })
  })

  it('give a key to one profile only', async () => {
    const shared = { keyA: await makeKey() }
    await registerDevice('alice', shared)
    expect((await registerDevice('bob', shared)).body).toEqual({ error: 'claimed' })

    const node = `nodekey:${'cd'.repeat(32)}`
    await registerDevice('alice', { keyA: await makeKey(), node })
    expect((await registerDevice('bob', { keyA: await makeKey(), node })).body).toEqual({ error: 'claimed' })
  })

  it('answer the same device when it registers again', async () => {
    const phone = { keyA: await makeKey() }
    const first = (await registerDevice('alice', phone)).body
    expect((await registerDevice('alice', phone, { label: 'Renamed' })).body).toEqual(first)
  })

  describe('certificates', () => {
    async function aliceWithPending() {
      const phone = { keyA: await makeKey() }
      const laptop = { keyA: await makeKey() }
      await registerDevice('alice', phone)
      const pending = (await registerDevice('alice', laptop)).body.device
      return { phone, laptop, pending }
    }

    it("must be signed by one of the same person's trusted devices", async () => {
      const { laptop, pending } = await aliceWithPending()
      const stranger = await makeKey()
      expect((await as('alice', 'devices/certify', { device: pending, certificate: await certificate('alice', stranger, laptop) })).body).toEqual({ error: 'certificate' })

      // Bob's trusted device can't vouch for Alice's.
      const bobPhone = { keyA: await makeKey() }
      await registerDevice('bob', bobPhone)
      expect((await as('alice', 'devices/certify', { device: pending, certificate: await certificate('alice', bobPhone.keyA, laptop) })).body).toEqual({ error: 'certificate' })
      expect((await env.DB.prepare(`SELECT state FROM device_keys WHERE id = ?`).bind(pending).first())).toEqual({ state: 'pending' })
    })

    it("can't be signed by the pending device itself, or another pending one", async () => {
      const { laptop, pending } = await aliceWithPending()
      expect((await as('alice', 'devices/certify', { device: pending, certificate: await certificate('alice', laptop.keyA, laptop) })).body).toEqual({ error: 'certificate' })
      const tablet = { keyA: await makeKey() }
      await registerDevice('alice', tablet)
      expect((await as('alice', 'devices/certify', { device: pending, certificate: await certificate('alice', tablet.keyA, laptop) })).body).toEqual({ error: 'certificate' })
    })

    it("can't be signed by a revoked device", async () => {
      const { phone, laptop, pending } = await aliceWithPending()
      const phoneId = (await as('alice', 'me')).body.devices[0].id
      expect((await as('alice', 'devices/revoke', { device: phoneId })).body).toEqual({ ok: true })
      expect((await as('alice', 'devices/certify', { device: pending, certificate: await certificate('alice', phone.keyA, laptop) })).body).toEqual({ error: 'certificate' })
    })

    it('must name the device it certifies', async () => {
      const { phone, pending } = await aliceWithPending()
      const other = { keyA: await makeKey() }
      expect((await as('alice', 'devices/certify', { device: pending, certificate: await certificate('alice', phone.keyA, other) })).body).toEqual({ error: 'certificate' })
    })

    it("can't be applied by anyone but the device's owner", async () => {
      const { phone, laptop, pending } = await aliceWithPending()
      expect((await as('mallory', 'devices/certify', { device: pending, certificate: await certificate('alice', phone.keyA, laptop) })).status).toBe(404)
    })
  })

  it("don't trust a new device on first use once someone has had any", async () => {
    const phone = { keyA: await makeKey() }
    const id = (await registerDevice('alice', phone)).body.device
    await as('alice', 'devices/revoke', { device: id })
    expect((await registerDevice('alice', { keyA: await makeKey() })).body.state).toBe('pending')
  })

  it('keep a revoked key from coming back', async () => {
    const phone = { keyA: await makeKey() }
    const id = (await registerDevice('alice', phone)).body.device
    expect((await as('mallory', 'devices/revoke', { device: id })).status).toBe(404)
    await as('alice', 'devices/revoke', { device: id })
    expect((await registerDevice('alice', phone)).body).toEqual({ error: 'revoked' })
    const lines = await env.DB.prepare(`SELECT action FROM audit WHERE actor = 'alice' ORDER BY id`).all()
    expect(lines.results).toEqual([{ action: 'device.registered' }, { action: 'device.revoked' }])
  })
})

// MARK: - Audit

describe('the audit log', () => {
  it('is for admins, newest first', async () => {
    const team = await teamWith('alice', ['bob'])
    expect((await as('bob', 'teams/audit', { team })).status).toBe(403)
    const log = await as('alice', 'teams/audit', { team })
    expect(log.body.entries.map((e: any) => e.action)).toEqual(['member.joined', 'invite.created', 'team.created'])
    expect(log.body.entries[0]).toMatchObject({ actor: 'bob', subject: 'bob', detail: { role: 'member' } })
  })
})

describe('routing', () => {
  it('answers an unknown directory route with 404', async () => {
    expect((await as('alice', 'teams/nope')).status).toBe(404)
    expect((await as('alice', 'constructor')).status).toBe(404)
  })
})
