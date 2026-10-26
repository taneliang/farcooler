import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker, { visitorKey } from '../src/index'

// The web client's Worker reaches this relay from Cloudflare's addresses, so
// keyed on the address alone all of its visitors shared one AUTH_LIMIT bucket,
// and a few could lock every web user out of sign-in and refresh (ov-811, S1).
// The Worker now names each visitor in a header signed with VISITOR_SECRET.

const SECRET = 'test-visitor-secret'
/// One Cloudflare egress address, shared by every call from the Worker.
const WORKER_IP = '2a06:98c0:3600::103'
/// Two visitors, as services/common/relay.ts names them, and their MACs under
/// SECRET. The Worker's suite (services/web/test/visitor-limits.test.ts) checks
/// that it signs these same two values, so the halves can't drift apart.
const ALICE = '2001:0db8:0001:0002::/64'
const ALICE_MAC = '9e080092fde7aae12e76d5e0773afeb413fd5ffbf740a0b03f1e00e3c60bd2bc'
const BOB = '203.0.113.7'
const BOB_MAC = '75bac095538e4692a2850ffe885d84e0011a3d6599197a0d686f28524bb36eb8'

/// The binding as workerd gives it: `allowed` calls per key.
function limiter(allowed: number) {
  const used = new Map<string, number>()
  const asked: string[] = []
  return {
    asked,
    async limit({ key }: { key: string }) {
      asked.push(key)
      used.set(key, (used.get(key) ?? 0) + 1)
      return { success: used.get(key)! <= allowed }
    },
  }
}

let workos = 0

beforeEach(() => {
  workos = 0
  vi.stubGlobal('fetch', async () => {
    workos += 1
    return new Response('{}', { status: 400 })
  })
})

function signIn(bindings: Record<string, unknown>, headers: Record<string, string>): Promise<Response> {
  return worker.fetch(
    new Request('https://relay.test/v1/auth/token', {
      method: 'POST',
      headers: { 'cf-connecting-ip': WORKER_IP, ...headers },
      body: JSON.stringify({ code: 'c', verifier: 'v' }),
    }),
    { ...env, ...bindings } as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
}

const as = (visitor: string, mac: string) => ({ 'x-fc-visitor': visitor, 'x-fc-visitor-mac': mac })

describe('the auth limit, per web visitor', () => {
  it('leaves another visitor signing in after one has used up their limit', async () => {
    const gate = limiter(2)
    const bindings = { AUTH_LIMIT: gate, VISITOR_SECRET: SECRET }

    expect((await signIn(bindings, as(ALICE, ALICE_MAC))).status).not.toBe(429)
    expect((await signIn(bindings, as(ALICE, ALICE_MAC))).status).not.toBe(429)
    expect((await signIn(bindings, as(ALICE, ALICE_MAC))).status).toBe(429)
    expect(workos).toBe(2)

    // Same address as Alice's calls, since both come through the Worker.
    expect((await signIn(bindings, as(BOB, BOB_MAC))).status).not.toBe(429)
    expect(workos).toBe(3)
    expect(gate.asked).toEqual([ALICE, ALICE, ALICE, BOB])
  })

  it('ignores a visitor header it cannot verify, and keys on the address', async () => {
    // Otherwise anyone could pick a fresh bucket per request, or spend someone
    // else's.
    const gate = limiter(100)
    const bindings = { AUTH_LIMIT: gate, VISITOR_SECRET: SECRET }
    const wrong = ALICE_MAC.replace(/^./, ALICE_MAC[0] === '0' ? '1' : '0')
    await signIn(bindings, as(ALICE, wrong))
    await signIn(bindings, as(BOB, ALICE_MAC))
    await signIn(bindings, as(ALICE, ALICE_MAC.toUpperCase()))
    await signIn(bindings, { 'x-fc-visitor': ALICE })
    await signIn(bindings, { 'x-fc-visitor-mac': ALICE_MAC })
    expect(gate.asked).toEqual(Array(5).fill(WORKER_IP))
  })

  it('ignores the header altogether when the relay has no secret', async () => {
    // An empty secret is no secret: an HMAC under "" is one anybody can make.
    for (const secret of [undefined, '', '  ']) {
      const gate = limiter(100)
      await signIn({ AUTH_LIMIT: gate, VISITOR_SECRET: secret }, as(ALICE, ALICE_MAC))
      expect(gate.asked, String(secret)).toEqual([WORKER_IP])
    }
  })

  it('keys a native app, which sends no visitor, on its address as before', async () => {
    const gate = limiter(100)
    await signIn({ AUTH_LIMIT: gate, VISITOR_SECRET: SECRET }, { 'cf-connecting-ip': '198.51.100.9' })
    expect(gate.asked).toEqual(['198.51.100.9'])
  })

  it('verifies the values the Worker signs', async () => {
    const check = (headers: Record<string, string>) =>
      visitorKey(new Request('https://relay.test/', { headers }), { ...env, VISITOR_SECRET: SECRET } as never)
    expect(await check(as(ALICE, ALICE_MAC))).toBe(ALICE)
    expect(await check(as(BOB, BOB_MAC))).toBe(BOB)
    expect(await check(as(`${ALICE}x`, ALICE_MAC))).toBeNull()
  })
})
