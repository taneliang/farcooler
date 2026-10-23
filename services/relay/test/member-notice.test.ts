import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { MEMBER_NOTICE_BODY_MAX, MEMBER_NOTICE_EVERY_MS, NEEDS_YOU_NOTICE_EVERY_MS } from '../src/member-notice'
import { RUNNER_NAMESPACE, runnerStatement } from '../src/directory/runners'
import { makeKey } from './sshsig-signer'

// `/v1/notify/member` (ov-646): a runner pushes one member of its team, in the
// relay's own words, at most once every six hours, and answers every refusal
// exactly as it answers a push. Against the real migration chain in workerd's D1.

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

beforeEach(() => {
  pushes = []
  vi.stubGlobal('fetch', async (input: any, init: any = {}) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    if (url.includes('oauth2.googleapis.com')) return new Response(JSON.stringify({ access_token: 'g', expires_in: 3600 }))
    const apple = url.includes('push.apple.com')
    if (apple || url.includes('fcm.googleapis.com')) {
      const body = JSON.parse(init.body)
      pushes.push(apple ? { token: url.split('/').pop()!, body } : { token: body.message.token, body: body.message })
    }
    return new Response('{}')
  })
})

const RELAY = 'https://relay.test'

/// A request, with every `waitUntil` awaited before it returns, so a push sent
/// after the response is counted. The raw response comes back with its text.
async function post(path: string, body: unknown, bearer?: string): Promise<{ status: number; text: string; type: string | null }> {
  const later: Promise<unknown>[] = []
  const response = await worker.fetch(
    new Request(`${RELAY}${path}`, { method: 'POST', headers: bearer ? { authorization: `Bearer ${bearer}` } : {}, body: JSON.stringify(body) }),
    env as never,
    { waitUntil: (p: Promise<unknown>) => later.push(p), passThroughOnException() {} } as never,
  )
  const text = await response.text()
  await Promise.all(later)
  return { status: response.status, text, type: response.headers.get('content-type') }
}

async function as(user: string, route: string, body: unknown = {}) {
  const r = await post(`/v1/directory/${route}`, body, await sessionFor(user))
  return { status: r.status, body: JSON.parse(r.text) }
}

/// A runner `owner` registers in team `team`, the way the daemon does: the box
/// signs. Returns its id and token.
async function runnerIn(owner: string, team: string, name: string): Promise<{ id: string; token: string }> {
  const install = crypto.randomUUID()
  const box = await makeKey(`box-${install}`)
  const issuedAt = Date.now()
  const made = await as(owner, 'runners/register', {
    team,
    install,
    name,
    runnerKey: box.line,
    issuedAt,
    proof: await box.sign(RUNNER_NAMESPACE, runnerStatement('register', owner, team, install, RELAY, issuedAt)),
  })
  expect(made.status).toBe(200)
  return { id: made.body.runner, token: made.body.token }
}

/// A team `admin` made, with each of `members` joined by invite as `member`.
async function teamWith(admin: string, members: string[], name = 'Acme'): Promise<string> {
  const team = (await as(admin, 'teams/create', { name })).body.team.id
  if (members.length) {
    const invite = await as(admin, 'teams/invites/create', { team, uses: members.length, role: 'member' })
    for (const member of members) expect((await as(member, 'invites/accept', { code: invite.body.code })).status).toBe(200)
  }
  return team
}

/// An app install signed in as `user`, with an email on the account so a test
/// can look for it in a payload.
async function install(user: string, token: string, platform = 'apns', notifyEvents: string | null = null) {
  await as(user, 'me')
  await env.DB.prepare(`UPDATE accounts SET email = ?, verified_email = ? WHERE id = ?`)
    .bind(`${user}@example.com`, `${user}@example.com`, user)
    .run()
  await env.DB.prepare(
    `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at, notify_events, notify_on_done)
     VALUES (?, ?, ?, ?, 'Phone', ?, ?, 0)`,
  )
    .bind(crypto.randomUUID(), user, platform, token, Date.now(), notifyEvents)
    .run()
}

let events = 0
const notice = (token: string, account: string, extra: Record<string, unknown> = {}) =>
  post('/v1/notify/member', { account, kind: 'login', event: `e${++events}`, ...extra }, token)

/// Alice owns the runner "build-box" in Acme; Bob and Carol are members.
/// Everyone has a phone.
async function acme() {
  const team = await teamWith('alice', ['bob', 'carol'])
  const runner = await runnerIn('alice', team, 'build-box')
  for (const user of ['alice', 'bob', 'carol']) await install(user, `${user}s-iphone`)
  return { team, runner }
}

async function audited(team: string): Promise<string[]> {
  const rows = await env.DB.prepare(`SELECT subject FROM audit WHERE team_id = ? AND action = 'runner.member_notified' ORDER BY id`)
    .bind(team)
    .all<{ subject: string }>()
  return rows.results.map(r => r.subject)
}

describe('a member whose login stopped', () => {
  it("gets one push on each of their devices, and nobody else's devices hear it", async () => {
    const { team, runner } = await acme()
    await install('bob', 'bobs-pixel', 'fcm')
    const sent = await notice(runner.token, 'bob', { runner: 'RUN-1' })
    expect(sent.status).toBe(200)
    expect(JSON.parse(sent.text)).toEqual({ ok: true })
    expect(pushes.map(p => p.token).sort()).toEqual(['bobs-iphone', 'bobs-pixel'])

    const apple = pushes.find(p => p.token === 'bobs-iphone')!.body
    expect(apple.aps.alert).toEqual({ title: 'Your Claude login on build-box stopped working.', body: 'Sign in again in Far Cooler.' })
    expect(apple.kind).toBe('login')
    expect(apple.runner).toBe('run-1')
    const android = pushes.find(p => p.token === 'bobs-pixel')!.body
    expect(android.notification.title).toBe('Your Claude login on build-box stopped working.')
    expect(android.android.notification.channel_id).toBe('account.devices')
    expect(android.data).toEqual({ kind: 'login', runner: 'run-1' })

    expect(await audited(team)).toEqual(['bob'])
  })

  it('carries no email, and no words of the request', async () => {
    const { runner } = await acme()
    await notice(runner.token, 'bob', { title: 'Click https://evil.example', subtitle: 'alice@example.com', email: 'x@y.z' })
    expect(pushes.length).toBe(1)
    const raw = JSON.stringify(pushes[0].body)
    expect(raw).not.toContain('@')
    expect(raw).not.toContain('evil')
  })

  it("names the runner from the relay's record, not the request", async () => {
    const { runner } = await acme()
    await notice(runner.token, 'bob', { name: 'Totally Real Bank' })
    expect(pushes[0].body.aps.alert.title).toBe('Your Claude login on build-box stopped working.')
  })

  it("reaches a device whose notification settings turned every class off", async () => {
    const team = await teamWith('alice', ['bob'])
    const runner = await runnerIn('alice', team, 'build-box')
    await install('bob', 'bobs-iphone', 'apns', '')
    await notice(runner.token, 'bob')
    expect(pushes.map(p => p.token)).toEqual(['bobs-iphone'])
  })
})

describe('the relay refuses with the answer a push gets', () => {
  it('to an outsider, the owner, a billing member, a removed member, an unknown account and an old-way token', async () => {
    const { team, runner } = await acme()
    await teamWith('mallory', [], 'Elsewhere')
    await install('mallory', 'mallorys-iphone')
    await install('dave', 'daves-iphone')
    await as('alice', 'teams/members/role', { team, user: 'carol', role: 'billing' })
    await teamWith('alice', ['erin'], 'Other')
    await install('erin', 'erins-iphone')
    // An admin who is not the owner may be told.
    await as('alice', 'teams/members/role', { team, user: 'bob', role: 'admin' })

    const member = await notice(runner.token, 'bob')
    expect(pushes.map(p => p.token)).toEqual(['bobs-iphone'])
    pushes = []

    // Paired the old way: a token with no runner record.
    const paired = await post('/v1/daemons', { label: 'This Mac' }, await sessionFor('alice'))
    const oldToken = JSON.parse(paired.text).token as string
    expect(oldToken).toBeTruthy()

    // Frank was in the team, and was removed.
    await install('frank', 'franks-iphone')
    const joined = await as('alice', 'teams/invites/create', { team, uses: 1, role: 'member' })
    expect((await as('frank', 'invites/accept', { code: joined.body.code })).status).toBe(200)
    expect((await as('alice', 'teams/members/remove', { team, user: 'frank' })).status).toBe(200)

    const refusals = {
      outsider: await notice(runner.token, 'mallory'),
      owner: await notice(runner.token, 'alice'),
      billing: await notice(runner.token, 'carol'),
      removed: await notice(runner.token, 'frank'),
      'in another of the owner’s teams': await notice(runner.token, 'erin'),
      'no such account': await notice(runner.token, 'user_nobody'),
      'an account in no team': await notice(runner.token, 'dave'),
      'old-way token': await notice(oldToken, 'bob'),
    }
    for (const [who, answer] of Object.entries(refusals)) expect(answer, who).toEqual(member)
    expect(pushes).toEqual([])
    expect(await audited(team)).toEqual(['bob'])
  })

  it("to an owner whose token's account is no longer the owner role", async () => {
    // Ownership passed (R-51): the role says Bob, the token still says Alice.
    const { runner } = await acme()
    await env.DB.prepare(`UPDATE runner_roles SET account_id = 'bob' WHERE runner_id = ? AND role = 'owner'`).bind(runner.id).run()
    await notice(runner.token, 'bob')
    await notice(runner.token, 'alice')
    expect(pushes).toEqual([])
  })

  it('and still refuses a malformed request out loud, whoever it names', async () => {
    const { runner } = await acme()
    expect((await post('/v1/notify/member', { account: 'bob', kind: 'other', event: 'e' }, runner.token)).text).toBe('{"error":"kind"}')
    expect((await post('/v1/notify/member', { account: 'bob', kind: 'login' }, runner.token)).text).toBe('{"error":"event"}')
    expect((await post('/v1/notify/member', { kind: 'login', event: 'e' }, runner.token)).text).toBe('{"error":"account"}')
    expect((await post('/v1/notify/member', { account: 'bob', kind: 'login', event: 'e' })).status).toBe(401)
    // A body past the cap, valid in every other way, is refused unread.
    const padded = { account: 'bob', kind: 'login', event: 'e-big', pad: 'x'.repeat(MEMBER_NOTICE_BODY_MAX) }
    expect((await post('/v1/notify/member', padded, runner.token)).text).toBe('{"error":"body"}')
    expect(pushes).toEqual([])
  })
})

describe('a member whose own item needs them (ov-633)', () => {
  const needsYou = (token: string, account: string) => notice(token, account, { kind: 'needs_you', runner: 'run-1' })

  it("gets the relay's words on each of their devices, on the blocked channel, and nobody else hears it", async () => {
    const { team, runner } = await acme()
    await install('bob', 'bobs-pixel', 'fcm')
    const sent = await needsYou(runner.token, 'bob')
    expect(JSON.parse(sent.text)).toEqual({ ok: true, sent: true })
    expect(pushes.map(p => p.token).sort()).toEqual(['bobs-iphone', 'bobs-pixel'])
    const apple = pushes.find(p => p.token === 'bobs-iphone')!.body
    expect(apple.aps.alert).toEqual({ title: 'Something on build-box needs you.', body: 'See it in Needs You.' })
    expect(apple.kind).toBe('needs_you')
    const android = pushes.find(p => p.token === 'bobs-pixel')!.body
    expect(android.android.notification.channel_id).toBe('agents.blocked')
    expect(android.data).toEqual({ kind: 'needs_you', runner: 'run-1' })
    // L2: no audit line for a push that can come every two minutes.
    expect(await audited(team)).toEqual([])
  })

  it("reaches only the member's devices that keep its class on, while a login push reaches every one", async () => {
    const team = await teamWith('alice', ['bob'])
    const runner = await runnerIn('alice', team, 'build-box')
    await install('bob', 'bobs-iphone', 'apns', 'decision')
    await install('bob', 'bobs-ipad', 'apns', '')
    await install('bob', 'bobs-pixel', 'fcm', null)
    await notice(runner.token, 'bob', { kind: 'needs_you', class: 'review' })
    expect(pushes.map(p => p.token)).toEqual(['bobs-pixel'])
    pushes = []
    await env.DB.prepare(`DELETE FROM member_notices`).run()
    await notice(runner.token, 'bob', { kind: 'needs_you', class: 'decision' })
    expect(pushes.map(p => p.token).sort()).toEqual(['bobs-iphone', 'bobs-pixel'])
    pushes = []
    await env.DB.prepare(`DELETE FROM member_notices`).run()
    // No class is a blocked agent's.
    await notice(runner.token, 'bob', { kind: 'needs_you' })
    expect(pushes.map(p => p.token)).toEqual(['bobs-pixel'])
    pushes = []
    await notice(runner.token, 'bob')
    expect(pushes.map(p => p.token).sort()).toEqual(['bobs-ipad', 'bobs-iphone', 'bobs-pixel'])
    expect((await post('/v1/notify/member', { account: 'bob', kind: 'needs_you', class: 'done', event: 'x' }, runner.token)).text).toBe('{"error":"class"}')
  })

  it('is refused for the owner and an outsider with the answer a push gets', async () => {
    const { runner } = await acme()
    await teamWith('mallory', [], 'Elsewhere')
    await install('mallory', 'mallorys-iphone')
    expect(JSON.parse((await needsYou(runner.token, 'bob')).text)).toEqual({ ok: true, sent: true })
    pushes = []
    // Each refusal reads as a member with no phone: nothing was sent.
    const none = { status: 200, text: '{"ok":true,"sent":false}' }
    for (const who of ['alice', 'mallory', 'user_nobody']) {
      const answer = await needsYou(runner.token, who)
      expect({ status: answer.status, text: answer.text }, who).toEqual(none)
    }
    expect(pushes).toEqual([])
  })

  it('says sent only when a push went: not with no install, nor when every install keeps the class off', async () => {
    const team = await teamWith('alice', ['bob', 'carol'])
    const runner = await runnerIn('alice', team, 'build-box')
    // Bob has no phone at all.
    expect(JSON.parse((await needsYou(runner.token, 'bob')).text)).toEqual({ ok: true, sent: false })
    // Carol's phone keeps decisions only.
    await install('carol', 'carols-iphone', 'apns', 'decision')
    const review = await notice(runner.token, 'carol', { kind: 'needs_you', class: 'review' })
    expect(JSON.parse(review.text)).toEqual({ ok: true, sent: false })
    expect(pushes).toEqual([])
    // That took no slot: her decision a moment later goes.
    const decision = await notice(runner.token, 'carol', { kind: 'needs_you', class: 'decision' })
    expect(JSON.parse(decision.text)).toEqual({ ok: true, sent: true })
    expect(pushes.map(p => p.token)).toEqual(['carols-iphone'])
    // Throttled within the window: she was told, so it still says sent.
    pushes = []
    const again = await notice(runner.token, 'carol', { kind: 'needs_you', class: 'decision' })
    expect(JSON.parse(again.text)).toEqual({ ok: true, sent: true })
    expect(pushes).toEqual([])
  })

  it('is one buzz per two minutes, and its own count apart from a login', async () => {
    const { runner } = await acme()
    await needsYou(runner.token, 'bob')
    await needsYou(runner.token, 'bob')
    expect(pushes.length).toBe(1)
    // A login push isn't held back by a Needs You one, nor the other way.
    await notice(runner.token, 'bob')
    expect(pushes.length).toBe(2)
    await env.DB.prepare(`UPDATE member_notices SET sent_at = sent_at - ? WHERE kind = 'needs_you'`)
      .bind(NEEDS_YOU_NOTICE_EVERY_MS - 1_000)
      .run()
    await needsYou(runner.token, 'bob')
    expect(pushes.length).toBe(2)
    await env.DB.prepare(`UPDATE member_notices SET sent_at = sent_at - 1001 WHERE kind = 'needs_you'`).run()
    await needsYou(runner.token, 'bob')
    expect(pushes.length).toBe(3)
    expect(pushes[2].body.aps.alert.title).toBe('Something on build-box needs you.')
  })
})

describe('the limits', () => {
  it('send at most one push per member per runner every six hours', async () => {
    const { team, runner } = await acme()
    await notice(runner.token, 'bob')
    await notice(runner.token, 'bob')
    expect(pushes.length).toBe(1)
    // Carol is her own count.
    await notice(runner.token, 'carol')
    expect(pushes.map(p => p.token)).toEqual(['bobs-iphone', 'carols-iphone'])
    // Another runner in the team is its own count too.
    const other = await runnerIn('alice', team, 'gpu-box')
    await notice(other.token, 'bob')
    expect(pushes.length).toBe(3)

    // Six hours on, Bob may hear from the first runner again.
    await env.DB.prepare(`UPDATE member_notices SET sent_at = sent_at - ? WHERE runner_id = ?`)
      .bind(MEMBER_NOTICE_EVERY_MS - 60_000, runner.id)
      .run()
    await notice(runner.token, 'bob')
    expect(pushes.length).toBe(3)
    await env.DB.prepare(`UPDATE member_notices SET sent_at = sent_at - 60001 WHERE runner_id = ?`).bind(runner.id).run()
    await notice(runner.token, 'bob')
    expect(pushes.length).toBe(4)
    expect(await audited(team)).toEqual(['bob', 'carol', 'bob', 'bob'])
  })

  it('send nothing for an event id already used, even after six hours', async () => {
    const { runner } = await acme()
    const once = { account: 'bob', kind: 'login', event: 'stop-1' }
    const first = await post('/v1/notify/member', once, runner.token)
    await env.DB.prepare(`UPDATE member_notices SET sent_at = sent_at - ?`).bind(MEMBER_NOTICE_EVERY_MS + 1).run()
    const again = await post('/v1/notify/member', once, runner.token)
    expect(again).toEqual(first)
    expect(pushes.length).toBe(1)
    await post('/v1/notify/member', { ...once, event: 'stop-2' }, runner.token)
    expect(pushes.length).toBe(2)
  })
})
