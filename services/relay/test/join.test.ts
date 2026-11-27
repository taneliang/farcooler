// A runner joining its owner's account from its own terminal (ov-1490),
// against the real migration chain in workerd's D1: the ticket, the claim and
// the approval are all checked in SQL.

import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { RUNNER_NAMESPACE, runnerStatement } from '../src/directory/runners'
import fixture from './fixtures/ceremony-reply.json'
import { makeKey, type TestKey } from './sshsig-signer'

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


beforeEach(() => {
  vi.stubGlobal('fetch', async (input: any) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    return new Response('{}')
  })
})

const RELAY = 'https://relay.test'
const ctx = { waitUntil() {}, passThroughOnException() {} } as never

async function call(path: string, body: unknown, user?: string, method = 'POST'): Promise<{ status: number; body: any }> {
  const response = await worker.fetch(
    new Request(`${RELAY}${path}`, {
      method,
      headers: user ? { authorization: `Bearer ${await sessionFor(user)}` } : {},
      ...(method === 'POST' ? { body: JSON.stringify(body) } : {}),
    }),
    env as never,
    ctx,
  )
  return { status: response.status, body: await response.json() }
}

const CEREMONY = fixture.offer.ceremony
const ALICE = 'join-alice'

/// A signed-in person's ticket for the command their browser shows.
async function ticket(user = ALICE): Promise<{ ticket: string; team: string }> {
  expect((await call('/v1/directory/me', {}, user)).status).toBe(200)
  const made = await call('/v1/join/ticket', { ceremony: CEREMONY }, user)
  expect(made.status).toBe(200)
  return made.body
}

/// The fields `farcooler join` signs for `register`, by `box`.
async function proof(box: TestKey, claim: string, team: string, user = ALICE, name = 'studio') {
  const issuedAt = Date.now()
  return {
    claim,
    install: 'join-install',
    name,
    runnerKey: box.line,
    issuedAt,
    proof: await box.sign(RUNNER_NAMESPACE, runnerStatement('register', user, team, 'join-install', RELAY, issuedAt)),
  }
}

const runners = async () => (await env.DB.prepare(`SELECT name FROM runners ORDER BY name`).all<{ name: string }>()).results
const audited = async () => (await env.DB.prepare(`SELECT action FROM audit ORDER BY id`).all<{ action: string }>()).results.map(r => r.action)

beforeEach(async () => {
  for (const table of ['join_tickets', 'ceremony_replies', 'runner_roles', 'runners', 'daemons', 'audit']) {
    await env.DB.prepare(`DELETE FROM ${table}`).run()
  }
})

describe('farcooler join (ov-1490)', () => {
  it('posts the sealed reply under the ticket, and adds the runner only once the person approves', async () => {
    const t = await ticket()
    expect((await call('/v1/join/check', { ticket: t.ticket })).status).toBe(200)
    const replied = await call('/v1/join/reply', { ticket: t.ticket, sealed: fixture.sealed })
    expect(replied.status).toBe(200)
    // The ticket is spent: no second reply, and a check says so.
    expect((await call('/v1/join/reply', { ticket: t.ticket, sealed: fixture.sealed })).status).toBe(404)
    expect((await call('/v1/join/check', { ticket: t.ticket })).status).toBe(404)
    // The browser reads it as its own account's.
    const read = await call(`/v1/ceremony/reply/${CEREMONY}`, null, ALICE, 'GET')
    expect(read.body.replies.map((r: any) => [r.sealed, r.posted_by])).toEqual([[fixture.sealed, ALICE]])

    const box = await makeKey('runner')
    const early = await call('/v1/join/register', await proof(box, replied.body.claim, t.team))
    expect([early.status, early.body.error]).toEqual([409, 'waiting'])
    expect(await runners()).toEqual([])

    expect((await call('/v1/join/approve', { ceremony: CEREMONY }, ALICE)).status).toBe(200)
    const added = await call('/v1/join/register', await proof(box, replied.body.claim, t.team))
    expect(added.status).toBe(200)
    expect(added.body.token).toMatch(/^[0-9a-f]{64}$/)
    expect(await runners()).toEqual([{ name: 'studio' }])
    expect(await audited()).toEqual(['runner.join_approved', 'runner.registered'])
    // Once: the claim is spent with the runner.
    expect((await call('/v1/join/register', await proof(box, replied.body.claim, t.team))).status).toBe(404)
  })

  it('approves nothing without a session, for another account, or before a reply', async () => {
    const t = await ticket()
    expect((await call('/v1/join/ticket', { ceremony: CEREMONY })).status).toBe(401)
    expect((await call('/v1/join/approve', { ceremony: CEREMONY }, ALICE)).status).toBe(404)
    const claim = (await call('/v1/join/reply', { ticket: t.ticket, sealed: fixture.sealed })).body.claim
    expect((await call('/v1/join/approve', { ceremony: CEREMONY })).status).toBe(401)
    expect((await call('/v1/join/approve', { ceremony: CEREMONY }, 'join-mallory')).status).toBe(404)
    const box = await makeKey('runner')
    expect((await call('/v1/join/register', await proof(box, claim, t.team))).status).toBe(409)
    expect(await runners()).toEqual([])
  })

  it('takes no reply or runner without a live ticket and claim', async () => {
    const t = await ticket()
    for (const bad of ['0'.repeat(64), 'nope', undefined]) {
      expect((await call('/v1/join/reply', { ticket: bad, sealed: fixture.sealed })).status).toBe(404)
      expect((await call('/v1/join/register', { claim: bad })).status).toBe(404)
    }
    expect((await call('/v1/join/reply', { ticket: t.ticket, sealed: 'SHA256:clear' })).status).toBe(400)
    await env.DB.prepare(`UPDATE join_tickets SET expires_at = ?`).bind(Date.now() - 1).run()
    expect((await call('/v1/join/reply', { ticket: t.ticket, sealed: fixture.sealed })).status).toBe(404)
    expect((await env.DB.prepare(`SELECT COUNT(*) AS n FROM ceremony_replies`).first<{ n: number }>())?.n).toBe(0)
  })

  it("registers on the runner's own signature for that account and team, and a bad proof keeps the claim", async () => {
    const t = await ticket()
    const claim = (await call('/v1/join/reply', { ticket: t.ticket, sealed: fixture.sealed })).body.claim
    await call('/v1/join/approve', { ceremony: CEREMONY }, ALICE)
    const box = await makeKey('runner')
    const wrong = await call('/v1/join/register', await proof(box, claim, t.team, 'join-mallory'))
    expect([wrong.status, wrong.body.error]).toEqual([403, 'proof'])
    expect(await runners()).toEqual([])
    expect((await call('/v1/join/register', await proof(box, claim, t.team))).status).toBe(200)
  })

  it('keeps no ticket or claim in the clear', async () => {
    const t = await ticket()
    const claim = (await call('/v1/join/reply', { ticket: t.ticket, sealed: fixture.sealed })).body.claim
    const rows = JSON.stringify((await env.DB.prepare(`SELECT * FROM join_tickets`).all()).results)
    expect(rows).not.toContain(t.ticket)
    expect(rows).not.toContain(claim)
  })
})
