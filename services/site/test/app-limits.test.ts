import { afterEach, describe, expect, it, vi } from 'vitest'

import worker, { type Env } from '../src/index'

// The per-visitor limit on /v1/auth calls (APP_AUTH_LIMIT), and the guards on
// what the directory sends back.

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
const jwt = (claims: Record<string, unknown>) => `${b64({ alg: 'RS256' })}.${b64(claims)}.c2ln`
const ACCESS = jwt({ sub: 'user_ME', exp: Math.floor(Date.now() / 1000) + 600 })

/// The binding as workerd gives it: a fixed number of calls per key.
function limiter(allowed: number): RateLimit {
  const used = new Map<string, number>()
  return {
    limit: async ({ key }: { key: string }) => {
      used.set(key, (used.get(key) ?? 0) + 1)
      return { success: used.get(key)! <= allowed }
    },
  } as RateLimit
}

const base = {
  APPCAST_URL: 'x',
  ASSETS: { fetch: async () => new Response('page') },
  APP_RELAY_URL: 'https://relay.example',
  APP_WORKOS_CLIENT_ID: 'client_TEST',
}

function relay(routes: Record<string, () => Response>): string[] {
  const calls: string[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const path = new URL(String(input)).pathname
      calls.push(path)
      return routes[path]?.() ?? Response.json({ error: 'not found' }, { status: 404 })
    }),
  )
  return calls
}

const from = (ip: string, path: string, cookie: string) =>
  new Request(`https://www.farcooler.com${path}`, { headers: { 'cf-connecting-ip': ip, cookie } })

afterEach(() => vi.unstubAllGlobals())

describe('the per-visitor limit on sign-in and refresh', () => {
  it('stops one visitor replaying callbacks, and not the next', async () => {
    const env = { ...base, APP_AUTH_LIMIT: limiter(2) } as unknown as Env
    const calls = relay({ '/v1/auth/token': () => Response.json({ error: 'auth' }, { status: 401 }) })
    const pending = `__Host-fc_signin=${b64({ s: 'st', v: 've' })}`
    const junk = (ip: string) => worker.fetch(from(ip, '/app/callback?code=junk&state=st', pending), env)

    expect((await junk('203.0.113.1')).status).toBe(400)
    expect((await junk('203.0.113.1')).status).toBe(400)
    const limited = await junk('203.0.113.1')
    expect(limited.status).toBe(429)
    expect(await limited.text()).toContain('Try again in a minute')
    expect(calls.filter(c => c === '/v1/auth/token')).toHaveLength(2)

    const other = await junk('198.51.100.7')
    expect(other.status).toBe(400)
    expect(calls.filter(c => c === '/v1/auth/token')).toHaveLength(3)
  })

  it('stops one visitor replaying dead cookies into refreshes, and not the next', async () => {
    const env = { ...base, APP_AUTH_LIMIT: limiter(2) } as unknown as Env
    const calls = relay({
      '/v1/directory/me': () => Response.json({ error: 'unauthorized' }, { status: 401 }),
      '/v1/auth/refresh': () => Response.json({ error: 'auth' }, { status: 401 }),
    })
    const dead = `__Host-fc_session=${b64({ a: ACCESS, r: 'dead' })}`
    const visit = (ip: string) => worker.fetch(from(ip, '/app', dead), env)

    await visit('203.0.113.1')
    await visit('203.0.113.1')
    const limited = await visit('203.0.113.1')
    expect(limited.status).toBe(429)
    expect(await limited.text()).toContain('Try again in a minute')
    expect(calls.filter(c => c === '/v1/auth/refresh')).toHaveLength(2)

    const other = await visit('198.51.100.7')
    expect(other.status).toBe(200)
    expect(await other.text()).toContain('href="/app/sign-in"')
    expect(calls.filter(c => c === '/v1/auth/refresh')).toHaveLength(3)
  })

  it('shows the same page when the relay itself is out of room', async () => {
    const env = { ...base, APP_AUTH_LIMIT: limiter(100) } as unknown as Env
    relay({ '/v1/auth/token': () => Response.json({ error: 'slow down' }, { status: 429 }) })
    const res = await worker.fetch(from('203.0.113.1', '/app/callback?code=c&state=st', `__Host-fc_signin=${b64({ s: 'st', v: 've' })}`), env)
    expect(res.status).toBe(429)
    expect(await res.text()).toContain('Try again in a minute')
  })
})

describe('guards', () => {
  it('treats a relay that is not https as not configured', async () => {
    for (const url of ['http://relay.example', 'not a url', 'ftp://relay.example']) {
      const res = await worker.fetch(new Request('https://www.farcooler.com/app'), { ...base, APP_RELAY_URL: url } as unknown as Env)
      expect(res.status).toBe(503)
    }
  })

  it('renders a members list with a date past what Date holds, and a role that is an Object property', async () => {
    const env = base as unknown as Env
    relay({
      '/v1/directory/me': () => Response.json({ user: { id: 'u1', email: 'a@b.example' }, teams: [] }),
      '/v1/directory/teams/members': () =>
        Response.json({
          team: { id: 't', name: 'T', personal: false },
          members: [
            { user: 'u1', email: 'a@b.example', role: 'constructor', joinedAt: 1e300 },
            { user: 'u2', email: 'c@d.example', role: '__proto__', joinedAt: 8.64e15 + 1 },
          ],
        }),
    })
    const res = await worker.fetch(new Request('https://www.farcooler.com/app/teams/t', { headers: { cookie: `__Host-fc_session=${b64({ a: ACCESS, r: 'r' })}` } }), env)
    expect(res.status).toBe(200)
    const html = await res.text()
    expect(html).not.toContain('Joined')
    expect(html).not.toContain('native code')
    expect(html.match(/class="end">(\w+)</g)).toEqual(['class="end">Member<', 'class="end">Member<'])
  })
})

describe('naming the visitor to the relay (ov-811)', () => {
  // The relay limits /v1/auth/* by address, and this Worker's address is
  // Cloudflare's, shared with every other Worker. With VISITOR_SECRET it names
  // each visitor, signed, so the relay limits them one by one. The MACs are the
  // ones the relay's suite verifies (services/relay/test/visitor-limit.test.ts).
  const ALICE = '2001:0db8:0001:0002::/64'
  const ALICE_MAC = '9e080092fde7aae12e76d5e0773afeb413fd5ffbf740a0b03f1e00e3c60bd2bc'
  const BOB = '203.0.113.7'
  const BOB_MAC = '75bac095538e4692a2850ffe885d84e0011a3d6599197a0d686f28524bb36eb8'

  function signed(): { path: string; visitor: string | null; mac: string | null }[] {
    const sent: { path: string; visitor: string | null; mac: string | null }[] = []
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const headers = new Headers(init?.headers)
        sent.push({ path: new URL(String(input)).pathname, visitor: headers.get('x-fc-visitor'), mac: headers.get('x-fc-visitor-mac') })
        return Response.json({ error: 'unauthorized' }, { status: 401 })
      }),
    )
    return sent
  }

  const pending = `__Host-fc_signin=${b64({ s: 'st', v: 've' })}`
  const dead = `__Host-fc_session=${b64({ a: jwt({ sub: 'user_ME', exp: 1 }), r: 'dead' })}`

  it('signs the visitor on sign-in and on refresh when it has the secret', async () => {
    const env = { ...base, VISITOR_SECRET: 'test-visitor-secret' } as unknown as Env
    const sent = signed()
    await worker.fetch(from('2001:db8:1:2::abcd', '/app/callback?code=junk&state=st', pending), env)
    await worker.fetch(from(BOB, '/app', dead), env)
    expect(sent.filter(s => s.path.startsWith('/v1/auth/'))).toEqual([
      { path: '/v1/auth/token', visitor: ALICE, mac: ALICE_MAC },
      { path: '/v1/auth/refresh', visitor: BOB, mac: BOB_MAC },
    ])
  })

  it('names no one without the secret', async () => {
    const sent = signed()
    await worker.fetch(from(BOB, '/app/callback?code=junk&state=st', pending), base as unknown as Env)
    await worker.fetch(from(BOB, '/app', dead), base as unknown as Env)
    expect(sent.filter(s => s.path.startsWith('/v1/auth/'))).toHaveLength(2)
    expect(sent.every(s => s.visitor === null && s.mac === null)).toBe(true)
  })
})
