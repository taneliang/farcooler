import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { sha256 } from '../src/index'
import { MAX_KEYS } from '../src/directory/inventory'
import { RUNNER_NAMESPACE, runnerStatement } from '../src/directory/runners'
import fixture from '../../../test/fixtures/contracts/runner/heartbeat-inventory.json'
import { makeKey, type TestKey } from './sshsig-signer'

// The key inventory (ov-510): what a runner reports of its authorized_keys on
// its heartbeat, who may read it, and the four labels the directory puts on
// each key. Against the real migration chain in workerd's D1.

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

const RELAY = 'https://relay.test'

async function call(path: string, body: unknown, bearer?: string): Promise<{ status: number; body: any }> {
  const response = await worker.fetch(
    new Request(`${RELAY}${path}`, { method: 'POST', headers: bearer ? { authorization: `Bearer ${bearer}` } : {}, body: JSON.stringify(body) }),
    env as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
  return { status: response.status, body: await response.json() }
}

async function as(user: string, route: string, body: unknown = {}) {
  return await call(`/v1/directory/${route}`, body, await sessionFor(user))
}

const boxes = new Map<string, TestKey>()

/// A runner in a fresh team `user` administers, registered the way the daemon
/// will: the box signs. Returns its id and the token it beats with.
async function runnerFor(user: string, install: string): Promise<{ id: string; token: string; team: string }> {
  const team = (await as(user, 'teams/create', { name: `${user}'s team` })).body.team.id
  const box = await makeKey(`box-${install}`)
  boxes.set(install, box)
  const issuedAt = Date.now()
  const made = await as(user, 'runners/register', {
    team,
    install,
    name: install,
    runnerKey: box.line,
    issuedAt,
    proof: await box.sign(RUNNER_NAMESPACE, runnerStatement('register', user, team, install, RELAY, issuedAt)),
  })
  expect(made.status).toBe(200)
  return { id: made.body.runner, token: made.body.token, team }
}

/// A SHA-256 fingerprint, as `ssh-keygen -lf` prints one.
async function fp(seed: string): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(seed)))
  return `SHA256:${btoa(String.fromCharCode(...digest)).replaceAll('=', '')}`
}

const HASH_A = 'a'.repeat(64)
const HASH_B = 'b'.repeat(64)

type Key = { fingerprint: string; comment?: string; inside?: boolean; scope?: string; file?: string; options?: string }
const entry = (k: Key) => ({ comment: '', inside: false, scope: 'other', file: 'authorized_keys', ...k })

/// A heartbeat carrying an inventory: the hash alone, or with its list.
function beat(token: string, hash: string, keys?: Key[], inventory: Record<string, unknown> = {}) {
  return call('/v1/heartbeat', { beatEvery: 300, install: 'whatever', inventory: { hash, unparsed: 0, ...(keys ? { entries: keys.map(entry) } : {}), ...inventory } }, token)
}

const keysOf = async (user: string, runner: string) => await as(user, 'runners/keys', { runner })

/// A device the directory knows, with its Key A, in whatever state.
async function device(user: string, fingerprint: string, state = 'certified', createdAt = Date.now()) {
  await as(user, 'me')
  const id = crypto.randomUUID()
  await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO device_keys (id, account_id, label, key_a, key_a_fingerprint, state, created_at) VALUES (?, ?, 'Phone', ?, ?, ?, ?)`,
    ).bind(id, user, `ssh-ed25519 ${id}`, fingerprint, state, createdAt),
    env.DB.prepare(`INSERT INTO device_fingerprints (fingerprint, device_id) VALUES (?, ?)`).bind(fingerprint, id),
  ])
}

async function joinTeam(team: string, admin: string, user: string) {
  const invite = await as(admin, 'teams/invites/create', { team, uses: 1, role: 'member' })
  expect((await as(user, 'invites/accept', { code: invite.body.code })).status).toBe(200)
}

async function grant(runner: string, user: string, role = 'control') {
  await env.DB.prepare(
    `INSERT INTO runner_roles (runner_id, account_id, role, granted_by, signature, granted_at) VALUES (?, ?, ?, 'alice', 'sig', ?)`,
  ).bind(runner, user, role, Date.now()).run()
}

async function auditActions(team: string): Promise<string[]> {
  const rows = await env.DB.prepare(`SELECT action FROM audit WHERE team_id = ? ORDER BY id`).bind(team).all<{ action: string }>()
  return rows.results.map(r => r.action)
}

const labelsOf = (listed: { body: any }) =>
  Object.fromEntries(listed.body.keys.map((k: any) => [k.comment || k.fingerprint, [k.label, k.reason]]))

describe('the heartbeat carries a hash, and the list only when it is wanted', () => {
  it('asks for the list when the relay has none, takes it, and then needs only the hash', async () => {
    const runner = await runnerFor('alice', 'i1')
    const k = await fp('one')

    // A hash the relay has never seen: it asks.
    expect((await beat(runner.token, HASH_A)).body).toEqual({ ok: true, inventory: 'send' })
    expect((await keysOf('alice', runner.id)).body.reportedAt).toBeNull()

    // The list: taken, and not asked for again.
    expect((await beat(runner.token, HASH_A, [{ fingerprint: k, comment: 'me@laptop' }])).body).toEqual({ ok: true })
    const first = await keysOf('alice', runner.id)
    expect(first.body.keys.map((x: any) => x.comment)).toEqual(['me@laptop'])

    // The same hash alone: nothing asked, nothing replaced, only the clock moves.
    await env.DB.prepare(`UPDATE runner_inventory SET reported_at = 1 WHERE runner_id = ?`).bind(runner.id).run()
    expect((await beat(runner.token, HASH_A)).body).toEqual({ ok: true })
    const again = await keysOf('alice', runner.id)
    expect(again.body.keys).toEqual(first.body.keys)
    expect(again.body.reportedAt).toBeGreaterThan(1)

    // A different hash alone: the relay's copy is stale, and it says so.
    expect((await beat(runner.token, HASH_B)).body).toEqual({ ok: true, inventory: 'send' })
    expect((await keysOf('alice', runner.id)).body.keys).toEqual(first.body.keys)

    // The new list replaces the old whole.
    await beat(runner.token, HASH_B, [{ fingerprint: await fp('two'), comment: 'ci@build' }])
    expect((await keysOf('alice', runner.id)).body.keys.map((x: any) => x.comment)).toEqual(['ci@build'])
  })

  it('is the old heartbeat, untouched, without an inventory or with a bad one', async () => {
    const runner = await runnerFor('alice', 'i1')
    expect((await call('/v1/heartbeat', { beatEvery: 300 }, runner.token)).body).toEqual({ ok: true })
    for (const inventory of [null, 'x', { hash: 'short' }, { hash: 7 }, []]) {
      expect((await call('/v1/heartbeat', { beatEvery: 300, inventory }, runner.token)).body).toEqual({ ok: true })
    }
    expect((await env.DB.prepare(`SELECT COUNT(*) AS n FROM runner_inventory`).first())).toEqual({ n: 0 })
  })

  it('is not asked of a pairing that predates the directory, and stores nothing for it', async () => {
    await as('alice', 'me')
    await env.DB.prepare(`INSERT INTO daemons (id, account_id, token_hash, label, created_at) VALUES ('old', 'alice', ?, 'Mac', ?)`)
      .bind(await sha256('old-token'), Date.now())
      .run()
    expect((await beat('old-token', HASH_A, [{ fingerprint: await fp('x') }])).body).toEqual({ ok: true })
    expect((await env.DB.prepare(`SELECT COUNT(*) AS n FROM runner_inventory`).first())).toEqual({ n: 0 })
  })

  it('takes the runner daemon posts as the shared contract spells it', async () => {
    const runner = await runnerFor('alice', 'i1')
    expect((await call('/v1/heartbeat', fixture, runner.token)).body).toEqual({ ok: true })
    const listed = await keysOf('alice', runner.id)
    expect(listed.body.keys.map((k: any) => [k.comment, k.inside, k.scope])).toEqual([
      ['me@laptop', false, 'other'],
      ['ci@build', true, 'other'],
    ])
    expect(listed.body.keys[0].fingerprint).toBe('SHA256:kmYcvdi2GkPeWxB6XLjrZB8JHsy2Hm8luHMFp9GMvqk')
  })

  it('keeps what it can use from a messy report, and bounds the rest', async () => {
    const runner = await runnerFor('alice', 'i1')
    const good = await fp('good')
    const long = 'x'.repeat(1000)
    const crowd = await Promise.all(Array.from({ length: MAX_KEYS + 20 }, async (_, i) => ({ fingerprint: await fp(`k${i}`), inside: false })))
    await call(
      '/v1/heartbeat',
      {
        inventory: {
          hash: HASH_A,
          unparsed: 3,
          entries: [
            { fingerprint: good, comment: `a\nb\u0000${long}`, inside: true, scope: 'host_admin' },
            { fingerprint: 'not-a-fingerprint', inside: true },
            { fingerprint: good, inside: 'yes' },
            null,
            'text',
            { fingerprint: await fp('odd-scope'), inside: false, scope: 'root' },
          ],
        },
      },
      runner.token,
    )
    const listed = await keysOf('alice', runner.id)
    expect(listed.body.keys).toHaveLength(2)
    expect(listed.body.keys[0].comment).toMatch(/^a b {1}x+$/)
    expect(listed.body.keys[0].comment.length).toBe(200)
    expect(listed.body.keys[1].scope).toBe('other')
    expect(listed.body.unparsed).toBe(3)

    await call('/v1/heartbeat', { inventory: { hash: HASH_B, entries: crowd } }, runner.token)
    expect((await keysOf('alice', runner.id)).body.keys).toHaveLength(MAX_KEYS)
  })

  it('goes with its runner', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('x') }])
    expect((await as('alice', 'runners/remove', { runner: runner.id })).status).toBe(200)
    expect((await env.DB.prepare(`SELECT COUNT(*) AS n FROM runner_inventory`).first())).toEqual({ n: 0 })
  })
})

describe('the four labels', () => {
  it('say known, unknown, orphaned and planted, each for its own reason', async () => {
    const runner = await runnerFor('alice', 'i1')
    await joinTeam(runner.team, 'alice', 'bob')
    await joinTeam(runner.team, 'alice', 'carol')
    await joinTeam(runner.team, 'alice', 'erin')
    await as('dave', 'me')

    // Bob: a certified device and a role here.
    await device('bob', await fp('bob'))
    await grant(runner.id, 'bob')
    // Carol: a certified device, in the team, no role on this runner.
    await device('carol', await fp('carol'))
    // Dave: a certified device and a role, but he is not in the team.
    await device('dave', await fp('dave'))
    await grant(runner.id, 'dave')
    // Erin: a role, and a device that was revoked.
    await device('erin', await fp('erin'), 'revoked')
    await grant(runner.id, 'erin')

    const baseline: Key[] = [
      { fingerprint: await fp('bob'), comment: 'bob', inside: true, scope: 'control' },
      { fingerprint: await fp('carol'), comment: 'carol', inside: true },
      { fingerprint: await fp('dave'), comment: 'dave', inside: true },
      { fingerprint: await fp('erin'), comment: 'erin', inside: true },
      { fingerprint: await fp('old-key'), comment: 'old-key', inside: false },
      { fingerprint: await fp('pasted'), comment: 'pasted', inside: true },
    ]
    await beat(runner.token, HASH_A, baseline)
    const first = labelsOf(await keysOf('alice', runner.id))
    expect(first).toEqual({
      bob: ['known', 'granted'],
      carol: ['orphaned', 'no_grant'],
      dave: ['orphaned', 'not_in_team'],
      erin: ['orphaned', 'revoked_device'],
      // There from the first report: not news.
      'old-key': ['unknown', 'no_device'],
      pasted: ['unknown', 'no_device'],
    })

    // A key nobody claims, outside the fence, that was not there before; and
    // one that appears inside the fence, which Far Cooler writes to.
    await beat(runner.token, HASH_B, [
      ...baseline,
      { fingerprint: await fp('planted'), comment: 'planted', inside: false },
      { fingerprint: await fp('new-inside'), comment: 'new-inside', inside: true },
    ])
    const second = await keysOf('alice', runner.id)
    expect(labelsOf(second)).toEqual({
      ...first,
      planted: ['planted', 'no_device'],
      // Inside the fence is no hiding place: new is new.
      'new-inside': ['planted', 'no_device'],
    })
    expect(second.body.counts).toEqual({ known: 1, unknown: 2, orphaned: 3, planted: 2 })

    // A third report that still has it: it was first seen at the second, and
    // stays planted; the report doesn't reset the clock.
    await beat(runner.token, 'c'.repeat(64), [
      ...baseline,
      { fingerprint: await fp('planted'), comment: 'planted', inside: false },
    ])
    const third = await keysOf('alice', runner.id)
    expect(labelsOf(third).planted).toEqual(['planted', 'no_device'])
    expect(third.body.keys.find((k: any) => k.comment === 'planted').firstSeenAt)
      .toBe(second.body.keys.find((k: any) => k.comment === 'planted').firstSeenAt)
  })

  it('follow the directory as it changes, with no new report', async () => {
    const runner = await runnerFor('alice', 'i1')
    await joinTeam(runner.team, 'alice', 'bob')
    await device('bob', await fp('bob'))
    await grant(runner.id, 'bob')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('bob'), comment: 'bob', inside: true, scope: 'control' }])
    expect(labelsOf(await keysOf('alice', runner.id)).bob).toEqual(['known', 'granted'])

    await env.DB.prepare(`DELETE FROM runner_roles WHERE runner_id = ? AND account_id = 'bob'`).bind(runner.id).run()
    expect(labelsOf(await keysOf('alice', runner.id)).bob).toEqual(['orphaned', 'no_grant'])
  })

  it('call a device that is not yet certified unknown, since no signed grant could have admitted it', async () => {
    const runner = await runnerFor('alice', 'i1')
    await joinTeam(runner.team, 'alice', 'bob')
    await device('bob', await fp('bob'), 'pending')
    await grant(runner.id, 'bob')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('bob'), comment: 'bob', inside: true }])
    expect(labelsOf(await keysOf('alice', runner.id)).bob).toEqual(['unknown', 'uncertified_device'])
  })
})

describe('whose inventory is whose', () => {
  it("a runner's token writes only its own, whatever the body names", async () => {
    const a = await runnerFor('alice', 'install-a')
    const b = await runnerFor('bob', 'install-b')
    await beat(b.token, HASH_B, [{ fingerprint: await fp('b-key'), comment: 'b-key' }])

    // A's token, naming B every way the body can.
    const response = await call(
      '/v1/heartbeat',
      {
        install: 'install-b',
        runner: b.id,
        runnerId: b.id,
        runner_id: b.id,
        inventory: { hash: HASH_A, runner: b.id, runnerId: b.id, entries: [entry({ fingerprint: await fp('a-key'), comment: 'a-key' })] },
      },
      a.token,
    )
    expect(response.body).toEqual({ ok: true })

    expect((await keysOf('alice', a.id)).body.keys.map((k: any) => k.comment)).toEqual(['a-key'])
    expect((await keysOf('bob', b.id)).body.keys.map((k: any) => k.comment)).toEqual(['b-key'])
    const rows = await env.DB.prepare(`SELECT runner_id, hash FROM runner_inventory ORDER BY hash`).all<{ runner_id: string; hash: string }>()
    expect(rows.results).toEqual([
      { runner_id: a.id, hash: HASH_A },
      { runner_id: b.id, hash: HASH_B },
    ])
    // And A's token asking for "send" is about A's row, not B's.
    expect((await beat(a.token, HASH_B)).body).toEqual({ ok: true, inventory: 'send' })
  })

  it("nobody reads another team's runner, and a runner token reads nothing", async () => {
    const a = await runnerFor('alice', 'install-a')
    await beat(a.token, HASH_A, [{ fingerprint: await fp('a-key'), comment: 'a-key' }])
    await as('bob', 'me')

    // Another team's admin: the same 404 as for no such runner.
    expect(await keysOf('bob', a.id)).toEqual({ status: 404, body: { error: 'not found' } })
    expect(await keysOf('bob', 'no-such-runner')).toEqual({ status: 404, body: { error: 'not found' } })
    // A runner's own token is not a person's session.
    expect((await call('/v1/directory/runners/keys', { runner: a.id }, a.token)).status).toBe(401)
    expect((await call('/v1/directory/runners/keys', { runner: a.id })).status).toBe(401)
    // A billing member sees none of a team's runners.
    const invite = await as('alice', 'teams/invites/create', { team: a.team, uses: 1, role: 'billing' })
    await as('bob', 'invites/accept', { code: invite.body.code })
    expect((await keysOf('bob', a.id)).status).toBe(403)
  })

  it("a team's members can read it, and the read writes nothing", async () => {
    const a = await runnerFor('alice', 'install-a')
    await joinTeam(a.team, 'alice', 'bob')
    await beat(a.token, HASH_A, [{ fingerprint: await fp('a-key'), comment: 'a-key' }])
    const before = JSON.stringify(await env.DB.prepare(`SELECT * FROM runner_inventory`).all())
    const listed = await keysOf('bob', a.id)
    expect(listed.status).toBe(200)
    expect(listed.body.keys).toHaveLength(1)
    expect(JSON.stringify(await env.DB.prepare(`SELECT * FROM runner_inventory`).all())).toBe(before)
  })
})

/// A runner with `alice` its owner and `bob` a member holding `role`, with a
/// certified device whose Key A is `fp('bob')`.
async function withBob(role: string) {
  const runner = await runnerFor('alice', 'i1')
  await joinTeam(runner.team, 'alice', 'bob')
  await device('bob', await fp('bob'))
  await grant(runner.id, 'bob', role)
  return runner
}

const bobAt = async (where: Partial<Key>) => ({ fingerprint: await fp('bob'), comment: 'bob', inside: true, scope: 'control', ...where })

describe('known means a matching grant (H1)', () => {
  it.each([
    ['control', 'control', true, 'known', 'granted'],
    ['control', 'read', true, 'known', 'granted'],
    ['admin', 'host_admin', true, 'known', 'granted'],
    ['shell', 'shell', true, 'known', 'granted'],
    // More than the role covers.
    ['read', 'control', true, 'orphaned', 'exceeds_grant'],
    ['control', 'host_admin', true, 'orphaned', 'exceeds_grant'],
    ['control', 'shell', true, 'orphaned', 'exceeds_grant'],
    ['read', 'shell', true, 'orphaned', 'exceeds_grant'],
    // Not a line Far Cooler wrote, even inside the fence.
    ['control', 'other', true, 'orphaned', 'unmanaged_line'],
    // Bare above the fence is a shell, whatever the role.
    ['control', 'other', false, 'orphaned', 'outside_fence'],
    ['read', 'read', false, 'orphaned', 'outside_fence'],
    ['admin', 'control', false, 'orphaned', 'outside_fence'],
  ])('a %s role on a %s line (inside: %s) reads %s, %s', async (role, scope, inside, label, reason) => {
    const runner = await withBob(role)
    await beat(runner.token, HASH_A, [await bobAt({ scope, inside })])
    expect(labelsOf(await keysOf('alice', runner.id)).bob).toEqual([label, reason])
  })

  it('does not take authorized_keys2 for the fence', async () => {
    const runner = await withBob('control')
    await beat(runner.token, HASH_A, [await bobAt({ file: 'authorized_keys2' })])
    expect(labelsOf(await keysOf('alice', runner.id)).bob).toEqual(['orphaned', 'outside_fence'])
  })

  it('never calls a cert-authority line known, and says so', async () => {
    const runner = await withBob('admin')
    await beat(runner.token, HASH_A, [await bobAt({ scope: 'ca', options: 'cert-authority,principals="deploy"' })])
    const listed = await keysOf('alice', runner.id)
    expect(labelsOf(listed).bob).toEqual(['unknown', 'ca_key'])
    expect(listed.body.keys[0]).toMatchObject({ flagged: true, options: 'cert-authority,principals="deploy"' })
  })

  it('unflags only the known', async () => {
    const runner = await withBob('control')
    await beat(runner.token, HASH_A, [await bobAt({})])
    expect((await keysOf('alice', runner.id)).body.keys[0].flagged).toBe(false)
  })
})

describe('planted means not known and new (H2)', () => {
  /// A baseline, then a later report adding `added`.
  async function plant(added: Key, setup?: (runner: { id: string; team: string }) => Promise<void>) {
    const runner = await runnerFor('alice', 'i1')
    await setup?.(runner)
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('old'), comment: 'old', inside: true }])
    await beat(runner.token, HASH_B, [{ fingerprint: await fp('old'), comment: 'old', inside: true }, added])
    return labelsOf(await keysOf('alice', runner.id))
  }

  it('whichever side of the fence it sits on', async () => {
    for (const inside of [true, false]) {
      expect((await plant({ fingerprint: await fp('new'), comment: 'new', inside })).new).toEqual(['planted', 'no_device'])
    }
  })

  it('whoever claims it, if the claim came after the key: a throwaway account registering a planted key', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('old'), comment: 'old', inside: true }])
    await beat(runner.token, HASH_B, [{ fingerprint: await fp('old'), comment: 'old', inside: true }, { fingerprint: await fp('throwaway'), comment: 'throwaway', inside: true }])
    await device('mallory', await fp('throwaway'), 'trusted', Date.now() + 60_000)
    const labels = labelsOf(await keysOf('alice', runner.id))
    expect(labels.throwaway).toEqual(['planted', 'not_in_team'])
    expect(labels.old).toEqual(['unknown', 'no_device'])
  })

  it('a certified teammate with a role is not planted, even when new', async () => {
    const runner = await runnerFor('alice', 'i1')
    await joinTeam(runner.team, 'alice', 'bob')
    await device('bob', await fp('bob'))
    await grant(runner.id, 'bob')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('old'), comment: 'old', inside: true }])
    await beat(runner.token, HASH_B, [{ fingerprint: await fp('old'), comment: 'old', inside: true }, await bobAt({})])
    expect(labelsOf(await keysOf('alice', runner.id)).bob).toEqual(['known', 'granted'])
  })

  it('a key moved from inside the fence to outside after the baseline (T1)', async () => {
    const runner = await runnerFor('alice', 'i1')
    const k = await fp('mover')
    await beat(runner.token, HASH_A, [{ fingerprint: k, comment: 'mover', inside: true }])
    expect(labelsOf(await keysOf('alice', runner.id)).mover).toEqual(['unknown', 'no_device'])
    await beat(runner.token, HASH_B, [{ fingerprint: k, comment: 'mover', inside: false }])
    expect(labelsOf(await keysOf('alice', runner.id)).mover).toEqual(['planted', 'no_device'])
  })

  it('a new cert-authority line, and a rise in unreadable lines, are flagged', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('old'), comment: 'old' }], { unparsed: 1 })
    expect((await keysOf('alice', runner.id)).body.unparsedSinceBaseline).toBe(0)
    await beat(runner.token, HASH_B, [{ fingerprint: await fp('old'), comment: 'old' }, { fingerprint: await fp('ca'), comment: 'ca', scope: 'ca' }], { unparsed: 3 })
    const listed = await keysOf('alice', runner.id)
    expect(labelsOf(listed).ca).toEqual(['planted', 'ca_key'])
    expect(listed.body.unparsedSinceBaseline).toBe(2)
    expect(listed.body.unparsed).toBe(3)
  })
})

describe('when the runner cannot read its keys (M1)', () => {
  it('records the error, keeps the last list, and shows how stale it is', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('a'), comment: 'a' }])
    const first = await keysOf('alice', runner.id)
    expect(first.body).toMatchObject({ error: null, errorAt: null, lastSeenAt: expect.any(Number), reportedAt: expect.any(Number) })

    expect((await beat(runner.token, HASH_B, undefined, { error: 'damaged' })).body).toEqual({ ok: true })
    const failed = await keysOf('alice', runner.id)
    expect(failed.body).toMatchObject({ error: 'damaged', errorAt: expect.any(Number), reportedAt: first.body.reportedAt })
    expect(failed.body.keys).toEqual(first.body.keys)
    expect(await auditActions(runner.team)).toContain('runner.inventory_error')

    // Repeating it writes no second line; a clean report clears it.
    await beat(runner.token, HASH_B, undefined, { error: 'damaged' })
    expect((await auditActions(runner.team)).filter(a => a === 'runner.inventory_error')).toHaveLength(1)
    await beat(runner.token, HASH_B, [{ fingerprint: await fp('a'), comment: 'a' }])
    expect((await keysOf('alice', runner.id)).body.error).toBeNull()
    expect(await auditActions(runner.team)).toContain('runner.inventory_recovered')
  })

  it('clears on a hash-only beat that matches the list it holds', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('a'), comment: 'a' }])
    await beat(runner.token, HASH_B, undefined, { error: 'refused' })
    expect((await keysOf('alice', runner.id)).body.error).toBe('refused')
    expect((await beat(runner.token, HASH_A)).body).toEqual({ ok: true })
    expect((await keysOf('alice', runner.id)).body).toMatchObject({ error: null, errorAt: null })
  })

  it('takes the keys a damaged fence still lists, with the error', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('a'), comment: 'a' }], { error: 'damaged' })
    expect((await keysOf('alice', runner.id)).body).toMatchObject({ error: 'damaged', keys: [expect.objectContaining({ comment: 'a' })] })
  })

  it('does not baseline a runner whose first report is an error', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, undefined, { error: 'refused' })
    const none = await keysOf('alice', runner.id)
    expect(none.body).toMatchObject({ error: 'refused', reportedAt: null, keys: [] })
    // The first real list is the baseline: nothing in it is news.
    await beat(runner.token, HASH_B, [{ fingerprint: await fp('a'), comment: 'a' }])
    expect(labelsOf(await keysOf('alice', runner.id)).a).toEqual(['unknown', 'no_device'])
    // And an unknown error code is no error.
    await beat(runner.token, HASH_B, undefined, { error: 'rm -rf' })
    expect((await keysOf('alice', runner.id)).body.error).toBeNull()
  })
})

describe('bounds and honesty about them', () => {
  it('keeps the total when it cuts, and writes no disappearances for a cut list', async () => {
    const runner = await runnerFor('alice', 'i1')
    const crowd = await Promise.all(Array.from({ length: MAX_KEYS + 20 }, async (_, i) => ({ fingerprint: await fp(`k${i}`), inside: false })))
    await beat(runner.token, HASH_A, crowd, { total: crowd.length })
    const listed = await keysOf('alice', runner.id)
    expect(listed.body.keys).toHaveLength(MAX_KEYS)
    expect(listed.body.total).toBe(MAX_KEYS + 20)
    await beat(runner.token, HASH_B, crowd.slice(0, 5), { total: 5 })
    expect((await auditActions(runner.team)).filter(a => a === 'runner.key_disappeared')).toHaveLength(0)
  })

  it('strips C1, bidi and zero-width characters from comments and options', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('x'), comment: 'bob‮@laptop​\u0085', options: 'from="a⁦b"' }])
    const key = (await keysOf('alice', runner.id)).body.keys[0]
    expect(key.comment).toBe('bob @laptop  ')
    expect(key.options).toBe('from="a b"')
  })

  it('refuses a heartbeat body over the limit, and a bad one, without touching the row', async () => {
    const runner = await runnerFor('alice', 'i1')
    const big = await call('/v1/heartbeat', { name: 'x'.repeat(600 * 1024) }, runner.token)
    expect(big).toEqual({ status: 413, body: { error: 'too large' } })
    const raw = await worker.fetch(
      new Request(`${RELAY}/v1/heartbeat`, { method: 'POST', headers: { authorization: `Bearer ${runner.token}` }, body: '{nope' }),
      env as never,
      { waitUntil() {}, passThroughOnException() {} } as never,
    )
    expect(raw.status).toBe(400)
  })

  it('lets the beat land when keeping the inventory fails', async () => {
    const runner = await runnerFor('alice', 'i1')
    const broken = {
      ...env,
      DB: {
        prepare: (sql: string) => {
          if (sql.includes('runner_inventory')) throw new Error('boom')
          return env.DB.prepare(sql)
        },
        batch: env.DB.batch.bind(env.DB),
      },
    }
    const response = await worker.fetch(
      new Request(`${RELAY}/v1/heartbeat`, {
        method: 'POST',
        headers: { authorization: `Bearer ${runner.token}` },
        body: JSON.stringify({ beatEvery: 300, name: 'Beaten', inventory: { hash: HASH_A } }),
      }),
      broken as never,
      { waitUntil() {}, passThroughOnException() {} } as never,
    )
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ ok: true })
    expect((await env.DB.prepare(`SELECT name FROM daemons WHERE id = (SELECT daemon_id FROM runners WHERE id = ?)`).bind(runner.id).first())).toEqual({ name: 'Beaten' })
  })
})

describe('the audit log keeps who put what where (L5)', () => {
  it('writes a line when a key appears or disappears after the baseline, and none for the baseline', async () => {
    const runner = await runnerFor('alice', 'i1')
    const [a, b] = [await fp('a'), await fp('b')]
    await beat(runner.token, HASH_A, [{ fingerprint: a, comment: 'a', inside: true }])
    expect(await auditActions(runner.team)).toEqual(['team.created', 'runner.registered'])
    await beat(runner.token, HASH_B, [{ fingerprint: b, comment: 'b' }])
    expect(await auditActions(runner.team)).toEqual(['team.created', 'runner.registered', 'runner.key_appeared', 'runner.key_disappeared'])
    const lines = await env.DB.prepare(`SELECT action, subject, actor FROM audit WHERE action LIKE 'runner.key_%' ORDER BY id`).all<any>()
    expect(lines.results).toEqual([
      { action: 'runner.key_appeared', subject: b, actor: `runner:${runner.id}` },
      { action: 'runner.key_disappeared', subject: a, actor: `runner:${runner.id}` },
    ])
  })
})

describe('offboarding is not planting (M)', () => {
  /// Bob's key is in the baseline's file only after the baseline: a later
  /// report adds it, as Far Cooler does when a grant is applied.
  async function appears(setup: (runner: { id: string; team: string; token: string }) => Promise<void>, scope = 'control') {
    const runner = await runnerFor('alice', 'i1')
    await joinTeam(runner.team, 'alice', 'bob')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('old'), comment: 'old', inside: true }])
    await setup(runner)
    await beat(runner.token, HASH_B, [{ fingerprint: await fp('old'), comment: 'old', inside: true }, await bobAt({ scope })])
    return labelsOf(await keysOf('alice', runner.id)).bob
  }

  it('a revoked device reads orphaned', async () => {
    expect(await appears(async () => { await device('bob', await fp('bob'), 'revoked', 1) })).toEqual(['orphaned', 'revoked_device'])
  })

  it('a missing grant reads orphaned', async () => {
    expect(await appears(async () => { await device('bob', await fp('bob'), 'certified', 1) })).toEqual(['orphaned', 'no_grant'])
  })

  it('a member who left reads orphaned', async () => {
    expect(await appears(async r => {
      await device('bob', await fp('bob'), 'certified', 1)
      await grant(r.id, 'bob')
      await env.DB.prepare(`DELETE FROM memberships WHERE team_id = ? AND account_id = 'bob'`).bind(r.team).run()
    })).toEqual(['orphaned', 'not_in_team'])
  })

  it('but the same claims made after the key appeared read planted', async () => {
    const runner = await runnerFor('alice', 'i1')
    await joinTeam(runner.team, 'alice', 'bob')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('old'), comment: 'old', inside: true }])
    await beat(runner.token, HASH_B, [{ fingerprint: await fp('old'), comment: 'old', inside: true }, await bobAt({})])
    await device('bob', await fp('bob'), 'revoked', Date.now() + 60_000)
    expect(labelsOf(await keysOf('alice', runner.id)).bob).toEqual(['planted', 'revoked_device'])
  })
})

describe('an error report never rewrites the history', () => {
  const baseline = async () => [
    { fingerprint: await fp('a'), comment: 'a', inside: true, scope: 'control' },
    { fingerprint: await fp('b'), comment: 'b', inside: false },
    { fingerprint: await fp('c'), comment: 'c', inside: false, file: 'authorized_keys2' },
  ]

  it('a damaged fence lists everything as outside and plants nothing', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, await baseline())
    const before = (await keysOf('alice', runner.id)).body.keys

    // The damaged report: every key outside the fence.
    const all = (await baseline()).map(k => ({ ...k, inside: false }))
    await beat(runner.token, HASH_B, all, { error: 'damaged', errorFile: 'authorized_keys' })
    const during = await keysOf('alice', runner.id)
    expect(during.body).toMatchObject({ error: 'damaged', errorFile: 'authorized_keys' })
    expect(during.body.keys).toEqual(before)

    // Repaired: the same keys as at the baseline, none of them new.
    await beat(runner.token, HASH_A, await baseline())
    const after = await keysOf('alice', runner.id)
    expect(after.body.error).toBeNull()
    expect(after.body.keys.map((k: any) => k.label)).toEqual(['unknown', 'unknown', 'unknown'])
    expect(after.body.keys.map((k: any) => k.firstSeenAt)).toEqual(before.map((k: any) => k.firstSeenAt))
    expect((await auditActions(runner.team)).filter(a => a.startsWith('runner.key_'))).toEqual([])
  })

  it('a primary file too large to read plants nothing when it can be read again', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, await baseline())
    const [, , c] = await baseline()
    await beat(runner.token, HASH_B, [c], { error: 'too_large', errorFile: 'authorized_keys' })
    await beat(runner.token, HASH_A, await baseline())
    expect((await keysOf('alice', runner.id)).body.keys.map((k: any) => k.label)).toEqual(['unknown', 'unknown', 'unknown'])
  })

  it('an authorized_keys2 that cannot be read still counts as a read of the rest, and keeps its keys', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, await baseline())
    const first = (await keysOf('alice', runner.id)).body
    await env.DB.prepare(`UPDATE runner_inventory SET reported_at = 1 WHERE runner_id = ?`).bind(runner.id).run()
    const [a, b] = await baseline()
    await beat(runner.token, HASH_B, [a, b], { error: 'refused', errorFile: 'authorized_keys2' })
    const during = (await keysOf('alice', runner.id)).body
    expect(during).toMatchObject({ error: 'refused', errorFile: 'authorized_keys2' })
    expect(during.reportedAt).toBeGreaterThan(1)
    expect(during.keys.map((k: any) => k.comment)).toEqual(['a', 'b', 'c'])
    expect(during.keys.map((k: any) => k.firstSeenAt)).toEqual(first.keys.map((k: any) => k.firstSeenAt))
  })
})

describe('a key moves between files, and the audit is capped', () => {
  it('seen: authorized_keys to authorized_keys2 is a new key', async () => {
    const runner = await runnerFor('alice', 'i1')
    const k = await fp('mover')
    await beat(runner.token, HASH_A, [{ fingerprint: k, comment: 'mover' }])
    await beat(runner.token, HASH_B, [{ fingerprint: k, comment: 'mover', file: 'authorized_keys2' }])
    expect(labelsOf(await keysOf('alice', runner.id)).mover).toEqual(['planted', 'no_device'])
    expect((await auditActions(runner.team)).filter(a => a.startsWith('runner.key_'))).toEqual(['runner.key_appeared', 'runner.key_disappeared'])
  })

  it('writes at most 25 lines for one report', async () => {
    const runner = await runnerFor('alice', 'i1')
    await beat(runner.token, HASH_A, [{ fingerprint: await fp('old'), comment: 'old' }])
    const crowd = await Promise.all(Array.from({ length: 40 }, async (_, i) => ({ fingerprint: await fp(`n${i}`), comment: `n${i}` })))
    await beat(runner.token, HASH_B, crowd)
    const lines = (await auditActions(runner.team)).filter(a => a.startsWith('runner.key_'))
    expect(lines).toHaveLength(25)
    expect(lines.every(a => a === 'runner.key_appeared')).toBe(true)
  })
})

describe('a heartbeat that is too big (M)', () => {
  it('drops an oversized inventory and still records the beat', async () => {
    const runner = await runnerFor('alice', 'i1')
    const huge = Array.from({ length: 1500 }, (_, i) => ({ fingerprint: `SHA256:${String(i).padStart(43, 'a')}`, inside: false, comment: 'x'.repeat(200) }))
    const response = await call('/v1/heartbeat', { beatEvery: 300, name: 'Alive', inventory: { hash: HASH_A, entries: huge } }, runner.token)
    expect(response).toEqual({ status: 200, body: { ok: true } })
    expect((await env.DB.prepare(`SELECT name FROM daemons WHERE id = (SELECT daemon_id FROM runners WHERE id = ?)`).bind(runner.id).first())).toEqual({ name: 'Alive' })
    expect((await env.DB.prepare(`SELECT COUNT(*) AS n FROM runner_inventory`).first())).toEqual({ n: 0 })
  })

  it('refuses a beat that is big without its inventory, and one past the read limit', async () => {
    const runner = await runnerFor('alice', 'i1')
    expect((await call('/v1/heartbeat', { name: 'x'.repeat(20 * 1024) }, runner.token)).status).toBe(413)
    expect((await call('/v1/heartbeat', { name: 'x'.repeat(3 * 1024 * 1024) }, runner.token)).status).toBe(413)
  })

  it('stops reading at the limit even when the length is not declared', async () => {
    const runner = await runnerFor('alice', 'i1')
    const chunk = new TextEncoder().encode('x'.repeat(256 * 1024))
    let sent = 0
    const body = new ReadableStream({
      start(c) {
        c.enqueue(new TextEncoder().encode('{"inventory":{"pad":"'))
      },
      pull(c) {
        if (sent++ < 12) c.enqueue(chunk)
        else {
          c.enqueue(new TextEncoder().encode('"}}'))
          c.close()
        }
      },
    })
    const response = await worker.fetch(
      new Request(`${RELAY}/v1/heartbeat`, { method: 'POST', headers: { authorization: `Bearer ${runner.token}` }, body, duplex: 'half' } as RequestInit),
      env as never,
      { waitUntil() {}, passThroughOnException() {} } as never,
    )
    expect(response.status).toBe(413)
  })
})
