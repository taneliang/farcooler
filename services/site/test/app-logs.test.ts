import { afterEach, describe, expect, it, vi } from 'vitest'

import siteToml from '../wrangler.toml?raw'
import worker, { type Env } from '../src/index'

// A token must never reach the Worker's logs (Workers Logs keeps every line,
// observability is on in wrangler.toml) or any part of a response but the
// cookie that carries it. Every path that handles one runs here with the
// console watched, including the failures, where a stray `console.error(e)`
// would print a request that carried it.
//
// The same goes for an invite code (ov-524): it arrives in a join link's path,
// rides the HttpOnly cookie through sign-in, and is posted to the relay. It
// may be on the one page that shows a new link to the admin who made it, and
// on no log line, header or other page.

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
const ACCESS = `${b64({ alg: 'RS256' })}.${b64({ sub: 'user_ME', sid: 'session_1', exp: Math.floor(Date.now() / 1000) + 10 })}.QUNDRVNTLVNFQ1JFVA`
const REFRESH = 'REFRESH-SECRET-abcdef'
const NEW_ACCESS = 'NEW-ACCESS-SECRET.eyJ9.x'
const NEW_REFRESH = 'NEW-REFRESH-SECRET'
const CODE = 'c0de'.repeat(16)
const CSRF = 'CSRF-SECRET-0123456789'
const SECRETS = [ACCESS, REFRESH, NEW_ACCESS, NEW_REFRESH, 'QUNDRVNTLVNFQ1JFVA', 'verifier-SECRET', 'code-SECRET', CODE]
const session = `__Host-fc_session=${b64({ a: ACCESS, r: REFRESH, c: CSRF })}`
const pending = `__Host-fc_signin=${b64({ s: 'state', v: 'verifier-SECRET', j: CODE })}`
const joining = `__Host-fc_signin=${b64({ j: CODE })}`
const form = (path: string, fields: Record<string, string>, cookie = session) =>
  new Request(`https://www.farcooler.com${path}`, {
    method: 'POST',
    headers: { cookie, origin: 'https://www.farcooler.com', 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ csrf: CSRF, ...fields }).toString(),
  })
const env = {
  APPCAST_URL: 'x',
  ASSETS: { fetch: async () => new Response('page') },
  APP_RELAY_URL: 'https://relay.example',
  APP_WORKOS_CLIENT_ID: 'client_TEST',
} as unknown as Env

type Relay = (path: string, init?: RequestInit) => Response | Promise<Response>

/// Each way the relay can go wrong while a token is in hand.
const RELAYS: Record<string, Relay> = {
  'a network failure whose message carries the request': (path, init) => {
    throw new Error(`fetch failed: ${path} ${JSON.stringify(init)}`)
  },
  'a 500 that echoes the request': (_path, init) => new Response(`boom ${JSON.stringify(init)}`, { status: 500 }),
  'a refusal that echoes the token': (_path, init) =>
    Response.json({ error: 'unauthorized', echo: JSON.stringify(init) }, { status: 401 }),
  'a body that is not JSON': () => new Response('<html>not json', { status: 200 }),
  'a refresh that works': path =>
    path === '/v1/auth/refresh'
      ? Response.json({ accessToken: NEW_ACCESS, refreshToken: NEW_REFRESH })
      : Response.json({ user: { id: 'user_ME' }, teams: [] }),
}

/// The one response that shows a code, to the admin who just made it.
const SHOWS_CODE = new Set(['making an invite'])

const REQUESTS: [string, () => Request][] = [
  ['the teams page', () => new Request('https://www.farcooler.com/app', { headers: { cookie: session } })],
  ['a members page', () => new Request('https://www.farcooler.com/app/teams/t1', { headers: { cookie: session } })],
  ['the callback', () => new Request('https://www.farcooler.com/app/callback?code=code-SECRET&state=state', { headers: { cookie: pending } })],
  ['signing out', () => form('/app/sign-out', {})],
  ['opening a join link', () => new Request(`https://www.farcooler.com/app/join/${CODE}`, { headers: { cookie: session } })],
  ['the join page', () => new Request('https://www.farcooler.com/app/join', { headers: { cookie: `${session}; ${joining}` } })],
  ['the join page, signed out', () => new Request('https://www.farcooler.com/app/join', { headers: { cookie: joining } })],
  ['starting sign-in with a code waiting', () => new Request('https://www.farcooler.com/app/sign-in', { headers: { cookie: joining } })],
  ['joining', () => form('/app/join', {}, `${session}; ${joining}`)],
  ['making an invite', () => form('/app/teams/t1/invites', { role: 'member', days: '7', uses: '50' })],
  ['revoking an invite', () => form('/app/teams/t1/invites/i1/revoke', {})],
  ['removing a member', () => form('/app/teams/t1/members/u2/remove', {})],
  ['the remove page', () => new Request('https://www.farcooler.com/app/teams/t1/members/u2/remove', { headers: { cookie: session } })],
]

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('the platform’s own log of each request', () => {
  // Workers Logs' invocation log records the request URL, and a join link has
  // its invite code in the path. Only the app's own console lines are left on.
  it('is off, for the site and (by inheritance) for canary and preview', () => {
    expect(siteToml).toMatch(/\[observability\.logs\][^\[]*invocation_logs\s*=\s*false/)
    expect(siteToml).not.toMatch(/\[env\.[a-z]+\.observability/)
  })
})

describe('tokens and logs', () => {
  for (const [relayName, relay] of Object.entries(RELAYS)) {
    for (const [requestName, request] of REQUESTS) {
      it(`logs no token for ${requestName}, given ${relayName}`, async () => {
        vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => relay(new URL(String(input)).pathname, init)))
        const lines: string[] = []
        for (const level of ['log', 'info', 'warn', 'error', 'debug', 'trace'] as const) {
          vi.spyOn(console, level).mockImplementation((...args: unknown[]) => {
            lines.push(args.map(a => (a instanceof Error ? `${a.message} ${a.stack}` : typeof a === 'string' ? a : JSON.stringify(a))).join(' '))
          })
        }

        const res = await worker.fetch(request(), env)
        const body = await res.text()
        const headers = [...res.headers].filter(([name]) => name !== 'set-cookie').map(([n, v]) => `${n}: ${v}`)
        for (const secret of SECRETS) {
          expect(lines.join('\n')).not.toContain(secret)
          if (!(secret === CODE && SHOWS_CODE.has(requestName))) expect(body).not.toContain(secret)
          expect(headers.join('\n')).not.toContain(secret)
        }
      })
    }
  }
})
