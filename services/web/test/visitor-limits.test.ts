import { afterEach, describe, expect, it, vi } from 'vitest'

import { prefix64, visitorOf } from '../../common/relay'
import type { WebEnv } from '../src/index'
import { ACCESS, envWith, get, RELAY } from './helpers'

// Who a visitor is, for the sign-in and refresh limits (ov-811, S1): an IPv6
// visitor is their /64, and the relay is told which visitor each call is for.

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
const jwt = (claims: Record<string, unknown>) => `${b64({ alg: 'RS256' })}.${b64(claims)}.c2lnbmF0dXJl`

const SECRET = 'test-visitor-secret'
/// The relay's suite (services/relay/test/visitor-limit.test.ts) verifies these
/// same two MACs under SECRET.
const ALICE = '2001:0db8:0001:0002::/64'
const ALICE_MAC = '9e080092fde7aae12e76d5e0773afeb413fd5ffbf740a0b03f1e00e3c60bd2bc'
const BOB = '203.0.113.7'
const BOB_MAC = '75bac095538e4692a2850ffe885d84e0011a3d6599197a0d686f28524bb36eb8'

/// The binding as workerd gives it: `allowed` calls per key.
function limiter(allowed: number) {
  const used = new Map<string, number>()
  return {
    limit: async ({ key }: { key: string }) => {
      used.set(key, (used.get(key) ?? 0) + 1)
      return { success: used.get(key)! <= allowed }
    },
  } as RateLimit
}

interface Sent {
  path: string
  visitor: string | null
  mac: string | null
}

/// Play the relay: sign-in refuses the code, refresh the token, `me` the token.
function relay(): Sent[] {
  const sent: Sent[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input))
      expect(url.origin).toBe(RELAY)
      const headers = new Headers(init?.headers)
      sent.push({ path: url.pathname, visitor: headers.get('x-fc-visitor'), mac: headers.get('x-fc-visitor-mac') })
      return Response.json({ error: 'unauthorized' }, { status: 401 })
    }),
  )
  return sent
}

afterEach(() => vi.unstubAllGlobals())

const PENDING = `__Host-fc_signin=${b64({ s: 'st', v: 've' })}`
const callback = (ip: string, env: WebEnv) => get('/callback?code=junk&state=st', env, { headers: { 'cf-connecting-ip': ip, cookie: PENDING } })
const expired = `__Host-fc_session=${b64({ a: jwt({ sub: 'x', exp: 1 }), r: 'dead', c: 'csrf' })}`
const visit = (ip: string, env: WebEnv) => get('/', env, { headers: { 'cf-connecting-ip': ip, cookie: expired } })

describe('the per-visitor limit', () => {
  it('counts every address in one IPv6 /64 as one visitor, and not the next /64', async () => {
    const env = envWith({ APP_AUTH_LIMIT: limiter(2) })
    const sent = relay()
    expect((await callback('2001:db8:1:2::1', env)).status).toBe(400)
    expect((await callback('2001:db8:1:2:ffff:ffff:ffff:fffe', env)).status).toBe(400)
    expect((await callback('2001:0DB8:0001:0002:0:0:0:3', env)).status).toBe(429)
    expect(sent.filter(s => s.path === '/v1/auth/token')).toHaveLength(2)

    expect((await callback('2001:db8:1:3::1', env)).status).toBe(400)
    expect(sent.filter(s => s.path === '/v1/auth/token')).toHaveLength(3)
  })

  it('leaves another visitor refreshing after one has used up their limit', async () => {
    const env = envWith({ APP_AUTH_LIMIT: limiter(2) })
    const sent = relay()
    await visit('2001:db8:1:2::1', env)
    await visit('2001:db8:1:2::2', env)
    expect((await visit('2001:db8:1:2::3', env)).status).toBe(429)
    expect(sent.filter(s => s.path === '/v1/auth/refresh')).toHaveLength(2)

    const other = await visit('198.51.100.7', env)
    expect(other.status).toBe(200)
    expect(sent.filter(s => s.path === '/v1/auth/refresh')).toHaveLength(3)
  })
})

describe('naming the visitor to the relay', () => {
  it('signs the visitor on sign-in and on refresh', async () => {
    const env = envWith({ VISITOR_SECRET: SECRET })
    const sent = relay()
    await callback('2001:db8:1:2::abcd', env)
    await visit('203.0.113.7', env)
    expect(sent.filter(s => s.path.startsWith('/v1/auth/'))).toEqual([
      { path: '/v1/auth/token', visitor: ALICE, mac: ALICE_MAC },
      { path: '/v1/auth/refresh', visitor: BOB, mac: BOB_MAC },
    ])
  })

  it('names no one without a secret, and nowhere but the auth routes', async () => {
    const sent = relay()
    await callback('203.0.113.7', envWith())
    await visit('203.0.113.7', envWith({ VISITOR_SECRET: '  ' }))
    expect(sent.length).toBeGreaterThan(1)
    expect(sent.every(s => s.visitor === null && s.mac === null)).toBe(true)

    const signed = relay()
    const live = `__Host-fc_session=${b64({ a: ACCESS, r: 'r', c: 'csrf' })}`
    await get('/', envWith({ VISITOR_SECRET: SECRET }), { headers: { 'cf-connecting-ip': BOB, cookie: live } })
    expect(signed.map(s => s.path)).toContain('/v1/directory/me')
    expect(signed.filter(s => !s.path.startsWith('/v1/auth/')).every(s => s.visitor === null)).toBe(true)
  })
})

describe('visitorOf', () => {
  const of = (ip?: string) => visitorOf(new Request('https://x.test/', { headers: ip === undefined ? {} : { 'cf-connecting-ip': ip } }))

  it('keeps an IPv4 address whole', () => {
    expect(of('203.0.113.7')).toBe('203.0.113.7')
    expect(of('203.0.113.8')).toBe('203.0.113.8')
  })

  it('writes every spelling of one /64 the same way', () => {
    for (const ip of ['2001:db8:1:2::', '2001:db8:1:2::1', '2001:0db8:0001:0002:1:2:3:4', '2001:DB8:1:2:a:b:c:d', ' 2001:db8:1:2::9 ']) {
      expect(of(ip), ip).toBe(ALICE)
    }
    expect(of('2001:db8::1')).toBe('2001:0db8:0000:0000::/64')
    expect(of('::1')).toBe('0000:0000:0000:0000::/64')
  })

  it('treats an IPv4-mapped address as the IPv4 one', () => {
    expect(of('::ffff:203.0.113.7')).toBe('203.0.113.7')
  })

  it('keeps an address it cannot read as it is, and no address as unknown', () => {
    expect(prefix64('2001:db8::1::2')).toBeNull()
    expect(prefix64('1:2:3:4:5:6:7')).toBeNull()
    expect(prefix64('1:2:3:4:5:6:7:8:9')).toBeNull()
    expect(prefix64('2001:db8:1:2::g')).toBeNull()
    expect(of('2001:db8::1::2')).toBe('2001:db8::1::2')
    expect(of()).toBe('unknown')
    expect(of('')).toBe('unknown')
  })
})
