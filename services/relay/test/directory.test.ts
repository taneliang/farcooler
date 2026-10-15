import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker, { installKey } from '../src/index'
import { CERTIFY_NAMESPACE, certifyStatement, REGISTER_NAMESPACE, registerStatement } from '../src/directory/devices'
import { RUNNER_NAMESPACE, runnerKey, runnerStatement } from '../src/directory/runners'
import { parsePublicKey, verifySignature } from '../src/sshsig'
import daemonFixture from './fixtures/runner-proof.json'
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

/// A session with the claims a real WorkOS access token carries, which do not
/// include `email` (`workos.ts`): the email reaches the relay only from the
/// authenticate response, at sign-in and refresh (`signIn` below).
async function sessionFor(user: string): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const claims = {
    sub: user,
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

/// What WorkOS's authenticate endpoint answers with, for `signIn`.
let workosUser: Record<string, unknown> = {}

beforeEach(() => {
  workosUser = {}
  vi.stubGlobal('fetch', async (input: any) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    if (url.includes('/user_management/authenticate')) {
      return new Response(JSON.stringify({ access_token: 'a', refresh_token: 'r', user: workosUser }))
    }
    return new Response('{}')
  })
})

/// Sign in through the relay's real code exchange, with WorkOS answering for
/// `user`: the only way an email, verified or not, reaches the directory.
async function signIn(user: string, email = `${user}@example.test`, verified = true) {
  workosUser = { id: user, email, email_verified: verified }
  expect((await call('/v1/auth/token', { code: 'c', verifier: 'v' })).status).toBe(200)
}

/// The env routes run against: the suite's, unless `racing` swapped in one
/// that lets another request commit at a chosen moment.
let routeEnv: any = env

/// Run `during` with a D1 whose next `batch` containing a statement matching
/// `match` first lets `before` run to completion (against the real env), the
/// way a second request committing between a route's read and its write
/// would. Each check that matters must hold inside the write itself.
async function racing<T>(match: RegExp, before: () => Promise<void>, during: () => Promise<T>): Promise<T> {
  const real = env.DB
  const sqlOf = new WeakMap<object, string>()
  let fired = false
  const track = (statement: any, sql: string) => {
    sqlOf.set(statement, sql)
    const bind = statement.bind.bind(statement)
    statement.bind = (...values: unknown[]) => track(bind(...values), sql)
    return statement
  }
  const db = {
    prepare: (sql: string) => track(real.prepare(sql), sql),
    batch: async (statements: any[]) => {
      if (!fired && statements.some(st => match.test(sqlOf.get(st) ?? ''))) {
        fired = true
        routeEnv = env
        await before()
        routeEnv = { ...env, DB: db }
      }
      return await real.batch(statements)
    },
    exec: real.exec.bind(real),
    dump: (real as any).dump?.bind(real),
  }
  routeEnv = { ...env, DB: db }
  try {
    const out = await during()
    expect(fired).toBe(true)
    return out
  } finally {
    routeEnv = env
  }
}

/// The origin every test request is made to: what a runner signs as its relay.
const RELAY = 'https://relay.test'

async function call(path: string, body: unknown, bearer?: string): Promise<{ status: number; body: any }> {
  const response = await worker.fetch(
    new Request(`${RELAY}${path}`, {
      method: 'POST',
      headers: bearer ? { authorization: `Bearer ${bearer}` } : {},
      body: JSON.stringify(body),
    }),
    routeEnv as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
  return { status: response.status, body: await response.json() }
}

/// Call a directory route as `user`.
async function as(user: string, route: string, body: unknown = {}) {
  return await call(`/v1/directory/${route}`, body, await sessionFor(user))
}

/// One runner key per install id, as one box keeps one key: the same install
/// registered twice is signed by the same box.
const boxes = new Map<string, TestKey>()
async function boxKey(install: string): Promise<TestKey> {
  if (!boxes.has(install)) boxes.set(install, await makeKey(`runner-${install}`))
  return boxes.get(install)!
}

/// Register (or claim) a runner as `user`, with the box signing the statement
/// the way the daemon will: `fields` are the body, and `as` overrides any of
/// what is signed.
async function register(
  user: string,
  fields: { team?: string; install?: unknown; name?: string; [k: string]: unknown },
  action: 'register' | 'claim' = 'register',
  key?: TestKey,
) {
  const team = fields.team ?? (await personalTeam(user))
  const install = typeof fields.install === 'string' ? fields.install : 'x'
  const box = key ?? (await boxKey(install))
  const issuedAt = Date.now()
  return await as(user, `runners/${action}`, {
    runnerKey: box.line,
    issuedAt,
    proof: await box.sign(RUNNER_NAMESPACE, runnerStatement(action, user, team, install, RELAY, issuedAt)),
    ...fields,
  })
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
    await signIn('alice')
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
    const work = (await register('alice', { team, install: 'work-box', name: 'Build box' })).body.runner
    const home = (await register('bob', { install: 'bob-laptop' })).body.runner
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

  it('lets an admin, and only an admin, readmit someone removed', async () => {
    const team = await teamWith('alice', ['bob', 'carol'])
    await as('alice', 'teams/members/remove', { team, user: 'bob' })
    expect((await as('carol', 'teams/members/readmit', { team, user: 'bob' })).status).toBe(403)
    expect((await as('alice', 'teams/members/readmit', { team, user: 'bob', role: 'admin' })).status).toBe(400)
    expect((await as('alice', 'teams/members/readmit', { team, user: 'bob' })).body).toEqual({ ok: true })
    expect(await env.DB.prepare(`SELECT role FROM memberships WHERE team_id = ? AND account_id = 'bob'`).bind(team).first()).toEqual({ role: 'member' })
    expect(await env.DB.prepare(`SELECT COUNT(*) AS n FROM team_removals`).first()).toEqual({ n: 0 })
    expect(await auditActions(team)).toContain('member.readmitted')
  })

  it('lists the removed, with their email, for admins only, so an admin can readmit one', async () => {
    await signIn('bob', 'bob@acme.com')
    const team = await teamWith('alice', ['bob', 'carol'])
    expect((await as('alice', 'teams/members', { team })).body.removed).toEqual([])
    await as('alice', 'teams/members/remove', { team, user: 'bob' })
    const seen = (await as('alice', 'teams/members', { team })).body
    expect(seen.removed).toEqual([{ user: 'bob', email: 'bob@acme.com', removedAt: expect.any(Number) }])
    expect(seen.members.map((m: any) => m.user)).not.toContain('bob')
    // A member sees the people, and nothing about who was removed.
    expect('removed' in (await as('carol', 'teams/members', { team })).body).toBe(false)
    // Leaving on your own is not a removal.
    await as('carol', 'teams/members/remove', { team, user: 'carol' })
    expect((await as('alice', 'teams/members', { team })).body.removed.map((r: any) => r.user)).toEqual(['bob'])
  })

  it("won't readmit someone who was never removed", async () => {
    const team = await teamWith('alice')
    expect((await as('alice', 'teams/members/readmit', { team, user: 'stranger' })).status).toBe(404)
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

  /// Each refusal says which it is, so a person can be told in plain words;
  /// `preview` gives the same answers without spending a use or joining.
  it('name the creator on preview by their verified email only', async () => {
    await signIn('alice', 'alice@acme.com', false)
    const team = await teamWith('alice')
    const { code } = (await as('alice', 'teams/invites/create', { team })).body
    expect((await as('bob', 'invites/preview', { code })).body.invitedBy).toBeNull()
    await signIn('alice', 'alice@acme.com', true)
    expect((await as('bob', 'invites/preview', { code })).body.invitedBy).toBe('alice@acme.com')
  })

  it('say why a code does not work, on accept and on preview alike', async () => {
    await signIn('alice', 'alice@acme.com')
    const team = await teamWith('alice')
    const revoked = (await as('alice', 'teams/invites/create', { team })).body
    await as('alice', 'teams/invites/revoke', { team, invite: revoked.invite })
    const expired = (await as('alice', 'teams/invites/create', { team })).body
    await env.DB.prepare(`UPDATE invites SET expires_at = ? WHERE id = ?`).bind(Date.now() - 1, expired.invite).run()
    const spent = (await as('alice', 'teams/invites/create', { team, uses: 1 })).body
    await as('bob', 'invites/accept', { code: spent.code })
    const open = (await as('alice', 'teams/invites/create', { team, uses: 3 })).body

    const cases: [string, string][] = [
      [revoked.code, 'revoked'],
      [expired.code, 'expired'],
      [spent.code, 'used'],
      ['f'.repeat(64), 'unknown'],
    ]
    for (const [code, reason] of cases) {
      expect((await as('carol', 'invites/accept', { code })).body).toEqual({ error: 'not found', reason })
      expect((await as('carol', 'invites/preview', { code })).body).toEqual({ error: 'not found', reason })
    }
    // A preview spends nothing and joins no one.
    expect((await as('carol', 'invites/preview', { code: open.code })).body).toEqual({
      team: { id: team, name: 'Acme' },
      role: 'member',
      member: false,
      invitedBy: 'alice@acme.com',
    })
    expect(await env.DB.prepare(`SELECT uses FROM invites WHERE id = ?`).bind(open.invite).first()).toEqual({ uses: 0 })
    expect(await env.DB.prepare(`SELECT COUNT(*) AS n FROM memberships WHERE account_id = 'carol' AND team_id = ?`).bind(team).first()).toEqual({ n: 0 })
    // Someone already in is told so.
    expect((await as('bob', 'invites/preview', { code: open.code })).body).toEqual({ team: { id: team, name: 'Acme' }, role: 'member', member: true })
    // A removed person, and a wrong domain, are told apart from the rest.
    await as('alice', 'teams/members/remove', { team, user: 'bob' })
    expect((await as('bob', 'invites/preview', { code: open.code })).body).toEqual({ error: 'not found', reason: 'removed' })
    const local = (await as('alice', 'teams/invites/create', { team, domain: 'acme.com' })).body
    expect((await as('carol', 'invites/preview', { code: local.code })).body).toEqual({ error: 'domain' })
    expect((await call('/v1/directory/invites/preview', { code: open.code })).status).toBe(401)
  })

  /// The email is WorkOS's verified one from sign-in, never a token claim:
  /// the sessions here carry none, as real access tokens don't.
  it('hold to their domain, by the email WorkOS verified', async () => {
    const team = await teamWith('alice')
    const { code } = (await as('alice', 'teams/invites/create', { team, domain: 'Acme.com', uses: 5 })).body
    await signIn('bob', 'bob@evil.com')
    expect((await as('bob', 'invites/accept', { code })).body).toEqual({ error: 'domain' })
    await signIn('eve', 'eve@notacme.com')
    expect((await as('eve', 'invites/accept', { code })).status).toBe(403)
    // Right domain, not verified.
    await signIn('dan', 'dan@acme.com', false)
    expect((await as('dan', 'invites/accept', { code })).status).toBe(403)
    // Never signed in through the relay: nothing verified to go on.
    expect((await as('frank', 'invites/accept', { code })).status).toBe(403)
    await signIn('carol', 'Carol@Acme.com')
    expect((await as('carol', 'invites/accept', { code })).status).toBe(200)
  })

  it("keep the email sign-in stored when a session carries none", async () => {
    await signIn('alice', 'alice@acme.com')
    await as('alice', 'me')
    expect(await env.DB.prepare(`SELECT email, verified_email FROM accounts WHERE id = 'alice'`).first()).toEqual({
      email: 'alice@acme.com',
      verified_email: 'alice@acme.com',
    })
  })

  it('spend one use when the same person accepts twice at once', async () => {
    const team = await teamWith('alice')
    const { code } = (await as('alice', 'teams/invites/create', { team, uses: 5 })).body
    const both = await Promise.all([as('bob', 'invites/accept', { code }), as('bob', 'invites/accept', { code })])
    expect(both.map(r => r.status)).toEqual([200, 200])
    expect(await env.DB.prepare(`SELECT uses FROM invites`).first()).toEqual({ uses: 1 })
    expect((await auditActions(team)).filter(a => a === 'member.joined')).toHaveLength(1)
  })

  it("don't let a removed member back in, however many uses are left", async () => {
    const team = await teamWith('alice')
    const { code } = (await as('alice', 'teams/invites/create', { team, uses: 10, days: 30 })).body
    await as('bob', 'invites/accept', { code })
    await as('alice', 'teams/members/remove', { team, user: 'bob' })
    expect((await as('bob', 'invites/accept', { code })).body).toEqual({ error: 'not found', reason: 'removed' })
    // Nothing spent on the refusal.
    expect(await env.DB.prepare(`SELECT uses FROM invites`).first()).toEqual({ uses: 1 })
    // A fresh invite doesn't work either.
    const fresh = (await as('alice', 'teams/invites/create', { team })).body.code
    expect((await as('bob', 'invites/accept', { code: fresh })).status).toBe(404)
  })

  it('let someone who left on their own come back', async () => {
    const team = await teamWith('alice')
    const { code } = (await as('alice', 'teams/invites/create', { team, uses: 2 })).body
    await as('bob', 'invites/accept', { code })
    await as('bob', 'teams/members/remove', { team, user: 'bob' })
    expect((await as('bob', 'invites/accept', { code })).body.joined).toBe(true)
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
async function grant(runner: string, user: string, role: string, grantedAt = Date.now()) {
  await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES (?, ?) ON CONFLICT (id) DO NOTHING`).bind(user, Date.now()).run()
  await env.DB.prepare(
    `INSERT INTO runner_roles (runner_id, account_id, role, granted_by, signature, granted_at) VALUES (?, ?, ?, 'test', 'sig', ?)`,
  )
    .bind(runner, user, role, grantedAt)
    .run()
}

describe('registering a runner', () => {
  it('makes the registrant its owner, in their personal team by default', async () => {
    await signIn('alice')
    const made = await register('alice', { install: 'install-1', name: 'Laptop' })
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
    const { token } = (await register('alice', { install: 'install-1', name: 'Laptop' })).body
    const beat = await call('/v1/heartbeat', { install: 'install-1', name: 'Laptop', every: 300 }, token)
    expect(beat.status).toBe(200)
    expect((await call('/v1/heartbeat', { install: 'install-1' }, 'not-a-token')).status).toBe(401)
  })

  it.each([[undefined], [''], ['has space'], ['x'.repeat(65)], [42]])('requires an install id, refusing %j', async install => {
    expect((await register('alice', { install })).body).toEqual({ error: 'install' })
    expect((await env.DB.prepare(`SELECT COUNT(*) AS n FROM daemons`).first())).toEqual({ n: 0 })
  })

  it('never stores the raw install id', async () => {
    await register('alice', { install: 'raw-install-id-123' })
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
    await register('alice', { team, install: 'box' })
    expect((await register('bob', { team, install: 'box' })).body).toEqual({ error: 'registered' })
    expect((await register('mallory', { install: 'box' })).body).toEqual({ error: 'registered' })
  })

  it("won't move a runner to another team by registering, and tells its owner so apart from someone else's", async () => {
    await register('alice', { install: 'box' })
    const team = await teamWith('alice', ['bob'])
    // Its owner: "elsewhere", so the app can offer a move.
    expect((await register('alice', { team, install: 'box' })).body).toEqual({ error: 'elsewhere' })
    // Someone who holds no role on it, in a team they belong to: still "registered".
    expect((await register('bob', { team, install: 'box' })).body).toEqual({ error: 'registered' })
  })

  it("never tells an admin who isn't the owner that it's elsewhere: only the owner may move it", async () => {
    const team = await teamWith('alice', ['bob'])
    const { runner } = (await register('alice', { team, install: 'box' })).body
    await grant(runner, 'bob', 'admin')
    const other = await teamWith('bob')
    expect((await register('bob', { team: other, install: 'box' })).body).toEqual({ error: 'registered' })
    // Rotating in its own team is still his to do, and the owner still gets "elsewhere".
    expect((await register('bob', { team, install: 'box' })).status).toBe(200)
    expect((await register('alice', { team: await teamWith('alice'), install: 'box' })).body).toEqual({ error: 'elsewhere' })
  })

  it("keeps a rotated token under the owner's account, whoever rotates it", async () => {
    const team = await teamWith('alice', ['bob'])
    const { runner } = (await register('alice', { team, install: 'box' })).body
    await grant(runner, 'bob', 'admin')
    expect((await register('bob', { team, install: 'box' })).status).toBe(200)
    expect((await env.DB.prepare(`SELECT account_id FROM daemons`).all()).results).toEqual([{ account_id: 'alice' }])
  })

  /// R2: a rotation that read Bob as owner, committing after Bob's removal
  /// passed the runner to Carol, must not hand Bob the push channel back.
  it("can't hand a removed owner the push channel back by racing his removal", async () => {
    const team = await teamWith('alice', ['bob', 'carol'])
    const { runner } = (await register('bob', { team, install: 'box' })).body
    await grant(runner, 'carol', 'admin')
    const rotated = await racing(
      /UPDATE runners SET daemon_id/,
      async () => {
        expect((await as('alice', 'teams/members/remove', { team, user: 'bob' })).status).toBe(200)
      },
      () => register('carol', { team, install: 'box' }),
    )
    expect(rotated.status).toBe(200)
    const rows = await env.DB.prepare(`SELECT account_id FROM daemons`).all<{ account_id: string }>()
    expect(rows.results).toEqual([{ account_id: 'carol' }])
  })

  it("won't rotate for someone who lost their runner role mid-request", async () => {
    const team = await teamWith('alice', ['bob', 'carol'])
    const { runner, token } = (await register('bob', { team, install: 'box' })).body
    await grant(runner, 'carol', 'admin')
    const rotated = await racing(
      /UPDATE runners SET daemon_id/,
      async () => {
        expect((await as('bob', 'runners/roles/remove', { runner, user: 'carol' })).status).toBe(200)
      },
      () => register('carol', { team, install: 'box' }),
    )
    expect(rotated.body).toEqual({ error: 'registered' })
    expect((await call('/v1/heartbeat', { install: 'box' }, token)).status).toBe(200)
    expect(await env.DB.prepare(`SELECT COUNT(*) AS n FROM daemons`).first()).toEqual({ n: 1 })
  })

  it('leaves exactly one live token after two rotations at once', async () => {
    const first = (await register('alice', { install: 'box' })).body
    const both = await Promise.all([register('alice', { install: 'box' }), register('alice', { install: 'box' })])
    const live = []
    for (const token of [first.token, ...both.map(r => r.body.token)]) {
      if ((await call('/v1/heartbeat', { install: 'box' }, token)).status === 200) live.push(token)
    }
    expect(live).toHaveLength(1)
    expect(await env.DB.prepare(`SELECT COUNT(*) AS n FROM daemons`).first()).toEqual({ n: 1 })
  })

  it('rotates the token when its owner registers it again, and the old one stops', async () => {
    const first = (await register('alice', { install: 'box', name: 'Box' })).body
    const second = (await register('alice', { install: 'box', name: 'Box 2' })).body
    expect(second.runner).toBe(first.runner)
    expect((await call('/v1/heartbeat', { install: 'box' }, first.token)).status).toBe(401)
    expect((await call('/v1/heartbeat', { install: 'box' }, second.token)).status).toBe(200)
    expect(await auditActions(first.team)).toEqual(['runner.registered', 'runner.token_rotated'])
  })

  it('is for admins and members of the team only', async () => {
    const team = await teamWith('alice', ['bill'], 'billing')
    expect((await register('bill', { team, install: 'box' })).status).toBe(403)
    expect((await register('mallory', { team, install: 'box' })).status).toBe(404)
  })
})

describe('taking a runner role away', () => {
  it("is for a team admin, or the runner's owner or admin", async () => {
    const team = await teamWith('alice', ['bob', 'carol', 'dave'])
    const runner = (await register('bob', { team, install: 'box' })).body.runner
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
    const runner = (await register('bob', { team, install: 'box' })).body.runner
    expect((await as('alice', 'runners/roles/remove', { runner, user: 'bob' })).body).toEqual({ error: 'owner' })
  })

  it('looks like no runner at all to someone outside its team', async () => {
    const runner = (await register('alice', { install: 'box' })).body.runner
    expect((await as('mallory', 'runners/roles/remove', { runner, user: 'alice' })).status).toBe(404)
  })

  it('reports a runner with no owner or admin left as unclaimed, and its token dies', async () => {
    const team = await teamWith('alice', ['bob'])
    const { token } = (await register('bob', { team, install: 'box' })).body
    await as('alice', 'teams/members/remove', { team, user: 'bob' })
    const listed = await as('alice', 'teams/runners', { team })
    expect(listed.body.runners[0]).toMatchObject({ owner: null, unclaimed: true, access: [] })
    // Bob's phone no longer hears from the team's runner.
    expect((await call('/v1/heartbeat', { install: 'box' }, token)).status).toBe(401)
  })
})

describe('when a runner owner leaves (R-51)', () => {
  it('the earliest-added runner admin becomes owner, and the pushes follow', async () => {
    const team = await teamWith('alice', ['bob', 'carol', 'dave'])
    const { runner, token } = (await register('bob', { team, install: 'box' })).body
    await grant(runner, 'dave', 'admin', 2000)
    await grant(runner, 'carol', 'admin', 1000)
    await as('alice', 'teams/members/remove', { team, user: 'bob' })

    const listed = (await as('alice', 'teams/runners', { team })).body.runners[0]
    expect(listed.owner).toBe('carol')
    expect(await env.DB.prepare(`SELECT account_id FROM daemons`).all()).toMatchObject({ results: [{ account_id: 'carol' }] })
    // The runner's token still works: only whose phone it reaches changed.
    expect((await call('/v1/heartbeat', { install: 'box' }, token)).status).toBe(200)
    const line = await env.DB.prepare(`SELECT subject, detail FROM audit WHERE action = 'runner.owner_changed'`).first<{ subject: string; detail: string }>()
    expect(line?.subject).toBe('carol')
    expect(JSON.parse(line!.detail)).toEqual({ runner, from: 'bob' })
  })

  it('breaks a tie by account, never with two owners', async () => {
    const team = await teamWith('alice', ['bob', 'zed', 'carol'])
    const { runner } = (await register('bob', { team, install: 'box' })).body
    await grant(runner, 'zed', 'admin', 1000)
    await grant(runner, 'carol', 'admin', 1000)
    expect((await as('alice', 'teams/members/remove', { team, user: 'bob' })).status).toBe(200)
    expect((await as('alice', 'teams/runners', { team })).body.runners[0].owner).toBe('carol')
  })

  it('changes nothing when the removal is refused', async () => {
    const team = await teamWith('alice')
    const { runner } = (await register('alice', { team, install: 'box' })).body
    expect((await as('alice', 'teams/members/remove', { team, user: 'alice' })).body).toEqual({ error: 'last admin' })
    expect((await as('alice', 'teams/runners', { team })).body.runners[0]).toMatchObject({ id: runner, owner: 'alice' })
    expect(await env.DB.prepare(`SELECT COUNT(*) AS n FROM daemons`).first()).toEqual({ n: 1 })
  })
})

describe('proving a runner (B1)', () => {
  it("needs the runner's own signature, not just its install id", async () => {
    const team = await personalTeam('mallory')
    const issuedAt = Date.now()
    const box = await boxKey('box')
    const forged = await (await makeKey()).sign(RUNNER_NAMESPACE, runnerStatement('register', 'mallory', team, 'box', RELAY, issuedAt))
    const made = await as('mallory', 'runners/register', { install: 'box', runnerKey: box.line, issuedAt, proof: forged })
    expect(made.body).toEqual({ error: 'proof' })
  })

  it('refuses a statement dated too far ahead of the relay clock', async () => {
    const team = await personalTeam('alice')
    const box = await boxKey('box')
    const issuedAt = Date.now() + 5 * 60_000
    const proof = await box.sign(RUNNER_NAMESPACE, runnerStatement('register', 'alice', team, 'box', RELAY, issuedAt))
    expect((await as('alice', 'runners/register', { install: 'box', runnerKey: box.line, issuedAt, proof })).body).toEqual({ error: 'stale' })
  })

  it.each([
    ['for another account', (team: string, at: number) => runnerStatement('register', 'bob', team, 'box', RELAY, at)],
    ['for another team', (_: string, at: number) => runnerStatement('register', 'alice', 'other-team', 'box', RELAY, at)],
    ['for another install', (team: string, at: number) => runnerStatement('register', 'alice', team, 'other', RELAY, at)],
    ['as a claim', (team: string, at: number) => runnerStatement('claim', 'alice', team, 'box', RELAY, at)],
    ['for another relay', (team: string, at: number) => runnerStatement('register', 'alice', team, 'box', 'https://relay-preview.test', at)],
  ])('refuses a signature made %s', async (_, statement) => {
    const team = await personalTeam('alice')
    const box = await boxKey('box')
    const issuedAt = Date.now()
    const proof = await box.sign(RUNNER_NAMESPACE, statement(team, issuedAt))
    expect((await as('alice', 'runners/register', { install: 'box', runnerKey: box.line, issuedAt, proof })).body).toEqual({ error: 'proof' })
  })

  it('refuses a claim printed for another relay', async () => {
    const work = await teamWith('mallory')
    await register('mallory', { team: work, install: 'box' })
    const team = await personalTeam('alice')
    const box = await boxKey('box')
    const issuedAt = Date.now()
    const proof = await box.sign(RUNNER_NAMESPACE, runnerStatement('claim', 'alice', team, 'box', 'https://relay-preview.test', issuedAt))
    expect((await as('alice', 'runners/claim', { install: 'box', runnerKey: box.line, issuedAt, proof })).body).toEqual({ error: 'proof' })
  })

  it('refuses a statement more than ten minutes old', async () => {
    const team = await personalTeam('alice')
    const box = await boxKey('box')
    const issuedAt = Date.now() - 11 * 60 * 1000
    const proof = await box.sign(RUNNER_NAMESPACE, runnerStatement('register', 'alice', team, 'box', RELAY, issuedAt))
    expect((await as('alice', 'runners/register', { install: 'box', runnerKey: box.line, issuedAt, proof })).body).toEqual({ error: 'stale' })
  })

  it("lets nobody squat a runner by its install id: the real one still registers", async () => {
    // Mallory read Bob's install id out of `ps` and signs with her own key.
    const squat = await register('mallory', { install: 'bobs-box' }, 'register', await makeKey())
    expect(squat.status).toBe(200)
    const real = await register('bob', { install: 'bobs-box' })
    expect(real.status).toBe(200)
    expect(real.body.runner).not.toBe(squat.body.runner)
  })

  it('stores the runner key, public only', async () => {
    await register('alice', { install: 'box' })
    const row = await env.DB.prepare(`SELECT runner_key FROM runners`).first<{ runner_key: string }>()
    expect(row!.runner_key).toBe((await boxKey('box')).line.replace(' runner-box', ''))
  })
})

// The daemon signs this same fixture byte for byte
// (`crates/daemon/src/runner_key_tests.rs`), so neither side can change the
// statement, the namespace or the armor without turning the other red.
describe("the daemon's signature", () => {
  const f = daemonFixture
  const statement = () => runnerStatement('register', f.account, f.team, f.install, f.relay, f.issuedAt)

  it("is the statement and namespace the relay verifies, made by the daemon's key", async () => {
    expect(statement()).toBe(f.statement)
    const signer = await verifySignature(f.proof, RUNNER_NAMESPACE, statement())
    expect(signer?.text).toBe(f.publicKey)
    expect((await parsePublicKey(f.publicKey))?.fingerprint).toBe(signer?.fingerprint)
  })

  it('is refused as another statement', async () => {
    expect(await verifySignature(f.proof, RUNNER_NAMESPACE, runnerStatement('claim', f.account, f.team, f.install, RELAY, f.issuedAt))).toBeNull()
    expect(await verifySignature(f.proof, RUNNER_NAMESPACE, runnerStatement('register', 'other', f.team, f.install, RELAY, f.issuedAt))).toBeNull()
  })
})

describe('claiming a runner', () => {
  it('moves it to the claimant with a fresh token, dropping every role', async () => {
    const work = await teamWith('mallory', ['carol'])
    const squatted = (await register('mallory', { team: work, install: 'box' })).body
    await grant(squatted.runner, 'carol', 'control')

    const claimed = await register('bob', { install: 'box' }, 'claim')
    expect(claimed.body).toMatchObject({ runner: squatted.runner, team: await personalTeam('bob') })
    expect((await call('/v1/heartbeat', { install: 'box' }, squatted.token)).status).toBe(401)
    expect((await call('/v1/heartbeat', { install: 'box' }, claimed.body.token)).status).toBe(200)
    expect((await as('mallory', 'teams/runners', { team: work })).body.runners).toEqual([])
    const roles = await env.DB.prepare(`SELECT account_id, role FROM runner_roles`).all()
    expect(roles.results).toEqual([{ account_id: 'bob', role: 'owner' }])
    expect(await auditActions(work)).toContain('runner.claimed_away')
  })

  it("needs the runner's signature", async () => {
    await register('alice', { install: 'box' })
    expect((await register('mallory', { install: 'box' }, 'claim', await makeKey())).status).toBe(404)
    const team = await personalTeam('mallory')
    const issuedAt = Date.now()
    const box = await boxKey('box')
    const proof = await (await makeKey()).sign(RUNNER_NAMESPACE, runnerStatement('claim', 'mallory', team, 'box', RELAY, issuedAt))
    expect((await as('mallory', 'runners/claim', { install: 'box', runnerKey: box.line, issuedAt, proof })).body).toEqual({ error: 'proof' })
  })
})

describe('removing a runner', () => {
  it('is for team admins, and kills its token', async () => {
    const team = await teamWith('alice', ['bob'])
    const { runner, token } = (await register('bob', { team, install: 'box' })).body
    expect((await as('bob', 'runners/remove', { runner })).status).toBe(403)
    expect((await as('mallory', 'runners/remove', { runner })).status).toBe(404)
    expect((await as('alice', 'runners/remove', { runner })).body).toEqual({ ok: true })
    expect((await call('/v1/heartbeat', { install: 'box' }, token)).status).toBe(401)
    expect((await as('alice', 'teams/runners', { team })).body.runners).toEqual([])
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

    // In either slot: Alice's Key A can't be Bob's Key B.
    expect((await registerDevice('bob', { keyA: await makeKey(), keyB: shared.keyA })).body).toEqual({ error: 'claimed' })
  })

  /// A node key can't sign, so anyone who has seen yours could claim it: it
  /// must never stop your device registering (review S5).
  it("don't let an unproven node key block anyone's device", async () => {
    const node = `nodekey:${'cd'.repeat(32)}`
    await registerDevice('mallory', { keyA: await makeKey(), node })
    expect((await registerDevice('bob', { keyA: await makeKey(), node })).body.state).toBe('trusted')
  })

  it('keep a revoked key from coming back as a Key B', async () => {
    const phone = { keyA: await makeKey() }
    const id = (await registerDevice('alice', phone)).body.device
    await as('alice', 'devices/revoke', { device: id })
    expect((await registerDevice('alice', { keyA: await makeKey(), keyB: phone.keyA })).body).toEqual({ error: 'revoked' })
  })

  it('give a revoked key no second identity through another encoding of it', async () => {
    const phone = { keyA: await makeKey() }
    const id = (await registerDevice('alice', phone)).body.device
    await as('alice', 'devices/revoke', { device: id })
    // The same 32 bytes, with a byte-order mark in front of the algorithm name.
    const blob = Uint8Array.from(atob(phone.keyA.line.split(' ')[1]), c => c.charCodeAt(0))
    const bom = new Uint8Array([0, 0, 0, 14, 0xef, 0xbb, 0xbf, ...blob.subarray(4)])
    const line = `ssh-ed25519 ${btoa(String.fromCharCode(...bom))}`
    expect((await registerDevice('alice', phone, { keyA: line })).body).toEqual({ error: 'keyA' })
  })

  /// R3: the certificate covers the node key, so a new one can't ride on it.
  it('send a certified device back for certifying when its node key changes', async () => {
    const phone = { keyA: await makeKey() }
    const laptop = { keyA: await makeKey(), node: `nodekey:${'aa'.repeat(32)}` }
    await registerDevice('alice', phone)
    const id = (await registerDevice('alice', laptop)).body.device
    await as('alice', 'devices/certify', { device: id, certificate: await certificate('alice', phone.keyA, laptop) })

    const moved = await registerDevice('alice', { ...laptop, node: `nodekey:${'bb'.repeat(32)}` })
    expect(moved.body).toEqual({ device: id, state: 'pending' })
    expect(await env.DB.prepare(`SELECT node_key, certificate, certified_by FROM device_keys WHERE id = ?`).bind(id).first()).toEqual({
      node_key: `nodekey:${'bb'.repeat(32)}`,
      certificate: null,
      certified_by: null,
    })
    expect(await env.DB.prepare(`SELECT COUNT(*) AS n FROM audit WHERE action = 'device.node_changed'`).first()).toEqual({ n: 1 })
    // The same node key again changes nothing.
    await as('alice', 'devices/certify', { device: id, certificate: await certificate('alice', phone.keyA, { ...laptop, node: `nodekey:${'bb'.repeat(32)}` }) })
    expect((await registerDevice('alice', { ...laptop, node: `nodekey:${'bb'.repeat(32)}` })).body.state).toBe('certified')
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
      expect((await as('alice', 'devices/revoke', { device: phoneId })).body).toEqual({ ok: true, uncertified: 0 })
      expect((await as('alice', 'devices/certify', { device: pending, certificate: await certificate('alice', phone.keyA, laptop) })).body).toEqual({ error: 'certificate' })
    })

    /// R1: the voucher read as trusted, then revoked before the write.
    it('never come from a device revoked mid-certify (R-52)', async () => {
      const { phone, laptop, pending } = await aliceWithPending()
      const phoneId = (await as('alice', 'me')).body.devices[0].id
      const signed = await certificate('alice', phone.keyA, laptop)
      const result = await racing(
        /SET state = 'certified'/,
        async () => {
          expect((await as('alice', 'devices/revoke', { device: phoneId })).status).toBe(200)
        },
        () => as('alice', 'devices/certify', { device: pending, certificate: signed }),
      )
      expect(result.status).toBe(409)
      expect(await env.DB.prepare(`SELECT state, certified_by FROM device_keys WHERE id = ?`).bind(pending).first()).toEqual({
        state: 'pending',
        certified_by: null,
      })
      expect(await env.DB.prepare(`SELECT COUNT(*) AS n FROM audit WHERE action = 'device.certified'`).first()).toEqual({ n: 0 })
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

  it('take back every certificate a revoked device signed, down the chain (R-52)', async () => {
    const phone = { keyA: await makeKey() }
    const laptop = { keyA: await makeKey() }
    const tablet = { keyA: await makeKey() }
    const watch = { keyA: await makeKey() }
    const phoneId = (await registerDevice('alice', phone)).body.device
    const laptopId = (await registerDevice('alice', laptop)).body.device
    await as('alice', 'devices/certify', { device: laptopId, certificate: await certificate('alice', phone.keyA, laptop) })
    const tabletId = (await registerDevice('alice', tablet)).body.device
    await as('alice', 'devices/certify', { device: tabletId, certificate: await certificate('alice', laptop.keyA, tablet) })

    expect((await as('alice', 'devices/revoke', { device: phoneId })).body).toEqual({ ok: true, uncertified: 2 })
    const states = await env.DB.prepare(`SELECT id, state, certified_by, certificate FROM device_keys ORDER BY created_at`).all()
    expect(states.results).toEqual([
      { id: phoneId, state: 'revoked', certified_by: null, certificate: null },
      { id: laptopId, state: 'pending', certified_by: null, certificate: null },
      { id: tabletId, state: 'pending', certified_by: null, certificate: null },
    ])
    // The tablet can no longer vouch for anything.
    const watchId = (await registerDevice('alice', watch)).body.device
    expect((await as('alice', 'devices/certify', { device: watchId, certificate: await certificate('alice', tablet.keyA, watch) })).body).toEqual({ error: 'certificate' })
    const lines = await env.DB.prepare(`SELECT subject FROM audit WHERE action = 'device.uncertified' ORDER BY subject`).all()
    expect(lines.results.map((r: any) => r.subject).sort()).toEqual([laptopId, tabletId].sort())
  })

  it("leave another person's devices alone when one is revoked", async () => {
    const phone = { keyA: await makeKey() }
    const laptop = { keyA: await makeKey() }
    await registerDevice('bob', phone)
    const laptopId = (await registerDevice('bob', laptop)).body.device
    await as('bob', 'devices/certify', { device: laptopId, certificate: await certificate('bob', phone.keyA, laptop) })
    const alicePhone = (await registerDevice('alice', { keyA: await makeKey() })).body.device
    expect((await as('alice', 'devices/revoke', { device: alicePhone })).body.uncertified).toBe(0)
    expect(await env.DB.prepare(`SELECT state FROM device_keys WHERE id = ?`).bind(laptopId).first()).toEqual({ state: 'certified' })
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

// MARK: - Old tokens (ov-529)

/// A runner paired the old way, `/v1/daemons`, as the Mac did before runners
/// were registered; `install` is what its first beat would have recorded.
async function pairOldWay(user: string, install: string | undefined): Promise<string> {
  const paired = await call('/v1/daemons', { label: 'This Mac', install }, await sessionFor(user))
  expect(paired.status).toBe(200)
  return paired.body.token
}

/// A beat, naming `install`: what a runner does once its new token is written.
const beats = async (token: string, install = 'box') => (await call('/v1/heartbeat', { install }, token)).status
const listedFor = async (user: string) => (await call('/v1/account', {}, await sessionFor(user))).body.machines
const revocations = async () =>
  (await env.DB.prepare(`SELECT team_id, actor, subject, detail FROM audit WHERE action = 'runner.old_tokens_revoked'`).all()).results

// The old tokens go when the new token first beats, never at the register: a
// failed write of the token onto the box must not leave the runner silent.
describe('registering a runner that was paired the old way (ov-529)', () => {
  it('revokes the old token at the new token\'s first beat, and the account lists the runner once', async () => {
    const old = await pairOldWay('alice', 'box')
    expect(await beats(old)).toBe(200)
    const { token } = (await register('alice', { install: 'box' })).body
    expect(await listedFor('alice')).toHaveLength(2)
    expect(await beats(token)).toBe(200)
    expect(await beats(old)).toBe(401)
    expect(await listedFor('alice')).toHaveLength(1)
  })

  it('revokes nothing while the new token is unused, so a failed install leaves the runner beating', async () => {
    const old = await pairOldWay('alice', 'box')
    await register('alice', { install: 'box' })
    expect(await beats(old)).toBe(200)
    expect(await revocations()).toEqual([])
  })

  it("revokes nothing when the beat names some other runner's install", async () => {
    const old = await pairOldWay('alice', 'box')
    const { token } = (await register('alice', { install: 'box' })).body
    expect(await beats(token, 'other-box')).toBe(200)
    expect(await beats(old)).toBe(200)
    expect(await beats(token)).toBe(200)
    expect(await beats(old)).toBe(401)
  })

  it('finds an old token by the install id its first beat recorded', async () => {
    const old = await pairOldWay('alice', undefined)
    expect(await beats(old)).toBe(200)
    const { token } = (await register('alice', { install: 'box' })).body
    await beats(token)
    expect(await beats(old)).toBe(401)
  })

  it("rotating it revokes the old-way tokens under the owner's account, whoever rotates", async () => {
    const team = await teamWith('alice', ['bob'])
    const { runner, token: first } = (await register('alice', { team, install: 'box' })).body
    await beats(first)
    await grant(runner, 'bob', 'admin')
    const old = await pairOldWay('alice', 'box')
    const rotated = (await register('bob', { team, install: 'box' })).body
    expect(await beats(old)).toBe(200)
    await beats(rotated.token)
    expect(await beats(old)).toBe(401)
    expect(await listedFor('alice')).toHaveLength(1)
  })

  it("leaves another account's rows alone, even with the same install id", async () => {
    const theirs = await pairOldWay('mallory', 'box')
    await beats((await register('alice', { install: 'box' })).body.token)
    expect(await beats(theirs)).toBe(200)
    expect(await listedFor('mallory')).toHaveLength(1)
  })

  it('leaves a different runner of the same account alone', async () => {
    const other = await pairOldWay('alice', 'other-box')
    const unnamed = await pairOldWay('alice', undefined)
    await beats((await register('alice', { install: 'box' })).body.token)
    expect(await beats(other)).toBe(200)
    expect(await beats(unnamed)).toBe(200)
  })

  it("won't let a key holder revoke another runner's tokens by naming its install", async () => {
    const team = await teamWith('alice', ['bob'])
    const { runner } = (await register('alice', { team, install: 'box' })).body
    await grant(runner, 'bob', 'admin')
    const elsewhere = await pairOldWay('alice', 'other-box')
    // Bob can make the runner sign, naming a different install.
    const box = await boxKey('box')
    const issuedAt = Date.now()
    const proof = await box.sign(RUNNER_NAMESPACE, runnerStatement('register', 'bob', team, 'other-box', RELAY, issuedAt))
    const rotated = await as('bob', 'runners/register', { team, install: 'other-box', runnerKey: box.line, issuedAt, proof })
    expect(rotated.status).toBe(200)
    await beats(rotated.body.token, 'other-box')
    expect(await beats(elsewhere, 'other-box')).toBe(200)
  })

  it('does the same on a claim: the old owner and the claimant lose their old-way tokens, nobody else', async () => {
    const squatted = (await register('mallory', { install: 'box' })).body
    const hers = await pairOldWay('mallory', 'box')
    const mine = await pairOldWay('bob', 'box')
    const bystander = await pairOldWay('carol', 'box')
    const claimed = await register('bob', { install: 'box' }, 'claim')
    expect(await beats(squatted.token)).toBe(401)
    expect(await beats(hers)).toBe(200)
    expect(await beats(mine)).toBe(200)
    await beats(claimed.body.token)
    expect(await beats(hers)).toBe(401)
    expect(await beats(mine)).toBe(401)
    expect(await beats(bystander)).toBe(200)
  })

  it("keeps an earlier claim's victims when the runner is registered again before its first beat", async () => {
    await register('alice', { install: 'box' })
    const hers = await pairOldWay('alice', 'box')
    const his = await pairOldWay('bob', 'box')
    await register('bob', { install: 'box' }, 'claim')
    const rotated = (await register('bob', { install: 'box' })).body
    await beats(rotated.token)
    expect(await beats(hers)).toBe(401)
    expect(await beats(his)).toBe(401)
  })

  it("keeps the first claim's victims when a second claim lands before the first beat", async () => {
    await register('alice', { install: 'box' })
    const hers = await pairOldWay('alice', 'box')
    const his = await pairOldWay('bob', 'box')
    const theirs = await pairOldWay('carol', 'box')
    await register('bob', { install: 'box' }, 'claim')
    const second = (await register('carol', { install: 'box' }, 'claim')).body
    await beats(second.token)
    expect(await beats(hers)).toBe(401)
    expect(await beats(his)).toBe(401)
    expect(await beats(theirs)).toBe(401)
  })

  it("a claim naming a different install revokes no old-way token", async () => {
    await register('mallory', { install: 'box' })
    const hers = await pairOldWay('mallory', 'elsewhere')
    const mine = await pairOldWay('bob', 'elsewhere')
    const claimed = await register('bob', { install: 'elsewhere' }, 'claim', await boxKey('box'))
    expect(claimed.status).toBe(200)
    await beats(claimed.body.token, 'elsewhere')
    expect(await beats(hers, 'elsewhere')).toBe(200)
    expect(await beats(mine, 'elsewhere')).toBe(200)
  })

  it("won't touch a second runner that shares the install id, registered under another key", async () => {
    const one = (await register('alice', { install: 'box' })).body
    const two = (await register('alice', { install: 'box' }, 'register', await makeKey('second'))).body
    const old = await pairOldWay('alice', 'box')
    // Neither new token may retire anything while two keys claim the install.
    await beats(one.token)
    await beats(two.token)
    expect(await beats(old)).toBe(200)
    // Bob holds the first runner's key and claims it: the second keeps its token.
    const claimed = await register('bob', { install: 'box' }, 'claim')
    await beats(claimed.body.token)
    expect(await beats(two.token)).toBe(200)
    expect(await beats(old)).toBe(200)
  })

  it("won't touch a token another runner row points at, whatever its install id", async () => {
    const one = (await register('alice', { install: 'box' })).body
    // The second runner registered with another install id, then beat with 'box'.
    const two = (await register('alice', { install: 'second-box' })).body
    await beats(two.token, 'second-box')
    await env.DB.prepare(`UPDATE daemons SET install_id = ? WHERE id = (SELECT daemon_id FROM runners WHERE id = ?)`)
      .bind(await installKey('alice', 'box'), two.runner)
      .run()
    const claimed = await register('bob', { install: 'box' }, 'claim')
    await beats(claimed.body.token)
    expect(one.runner).toBe(claimed.body.runner)
    expect(await beats(two.token, 'second-box')).toBe(200)
  })

  it('revokes nothing when the runner was rotated between the beat and the delete', async () => {
    const old = await pairOldWay('alice', 'box')
    const first = (await register('alice', { install: 'box' })).body
    const rotated = await racing(
      /old_tokens_revoked/,
      async () => {
        expect((await register('alice', { install: 'box' })).status).toBe(200)
      },
      () => call('/v1/heartbeat', { install: 'box' }, first.token),
    )
    expect(rotated.status).toBe(200)
    expect(await beats(old)).toBe(200)
  })

  it('says so in the log, with the count, only when something was revoked', async () => {
    const team = await personalTeam('alice')
    const { token } = (await register('alice', { install: 'box' })).body
    await beats(token)
    expect(await revocations()).toEqual([])
    await pairOldWay('alice', 'box')
    await pairOldWay('alice', 'box')
    const again = (await register('alice', { install: 'box' })).body
    await beats(again.token)
    expect(await revocations()).toEqual([
      { team_id: team, actor: 'alice', subject: again.runner, detail: JSON.stringify({ account: 'alice', tokens: 2 }) },
    ])
  })

  it("logs a claim's revocations in the previous owner's team too", async () => {
    const work = await teamWith('mallory')
    const squatted = (await register('mallory', { team: work, install: 'box' })).body
    await pairOldWay('mallory', 'box')
    const claimed = (await register('bob', { install: 'box' }, 'claim')).body
    await beats(claimed.token)
    expect((await revocations()).map((r: any) => r.team_id)).toEqual([work])
    expect(squatted.runner).toBe(claimed.runner)
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

describe('the schema', () => {
  it('refuses an unsigned grant of anything but ownership', async () => {
    const { runner } = (await register('alice', { install: 'box' })).body
    await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES ('bob', 0)`).run()
    await expect(
      env.DB.prepare(`INSERT INTO runner_roles (runner_id, account_id, role, granted_at) VALUES (?, 'bob', 'control', 0)`).bind(runner).run(),
    ).rejects.toThrow(/CHECK constraint failed/)
  })
})

describe('routing', () => {
  it('answers an unknown directory route with 404', async () => {
    expect((await as('alice', 'teams/nope')).status).toBe(404)
    expect((await as('alice', 'constructor')).status).toBe(404)
  })
})
