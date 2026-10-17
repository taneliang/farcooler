import { afterEach, describe, expect, it, vi } from 'vitest'

import worker, { type Env } from '../src/index'

// The web app's forms as a real browser posts them (ov-602). Every /app page
// sets its own Referrer-Policy; Chrome, Safari and Firefox derive the headers
// of a form post from it. The first test lets the page's header decide, so it
// fails if the policy and the Origin check ever disagree again.

const RELAY = 'https://relay.example'
const ORIGIN = 'https://www.farcooler.com'
const ASSETS = { fetch: async () => new Response('marketing page') } as unknown as Fetcher
const env = { APPCAST_URL: 'x', ASSETS, APP_RELAY_URL: RELAY, APP_WORKOS_CLIENT_ID: 'client_TEST' } as Env

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
const jwt = (claims: Record<string, unknown>) => `${b64({ alg: 'RS256' })}.${b64(claims)}.c2lnbmF0dXJl`
const ACCESS = jwt({ sub: 'user_ME', sid: 'session_01ABC', exp: Math.floor(Date.now() / 1000) + 600 })
const CSRF = 'csrf-TEST-0123456789'
const SESSION = `__Host-fc_session=${b64({ a: ACCESS, r: 'refresh_SECRET', c: CSRF })}`

function relay() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const path = new URL(String(input)).pathname.replace('/v1/directory/', '')
      if (path === 'me') return Response.json({ user: { id: 'user_ME', email: 'me@example.com' }, teams: [], devices: [] })
      if (path === 'teams/create') return Response.json({ team: { id: 't1' } })
      return Response.json({ error: 'not found' }, { status: 404 })
    }),
  )
}

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

/// The headers a browser sends for a form post from `page`, given the policy
/// the page's own response set (Fetch: "serialize a request origin" and
/// "determine the request's referrer"). Under no-referrer: Origin: null and no
/// Referer. Under same-origin, to the same origin: the real Origin and Referer.
function browserPost(page: Response, pageUrl: string, path: string, fields: Record<string, string>) {
  const policy = page.headers.get('referrer-policy')
  const headers: Record<string, string> = {
    'content-type': 'application/x-www-form-urlencoded',
    cookie: SESSION,
    'sec-fetch-site': 'same-origin',
    'sec-fetch-mode': 'navigate',
    'sec-fetch-dest': 'document',
  }
  if (policy === 'no-referrer') headers.origin = 'null'
  else if (policy === 'same-origin') {
    headers.origin = ORIGIN
    headers.referer = pageUrl
  } else throw new Error(`the test models no browser for ${policy}`)
  return worker.fetch(new Request(`${ORIGIN}${path}`, { method: 'POST', headers, body: new URLSearchParams(fields).toString() }), env)
}

describe('a form post from an /app page, in a browser', () => {
  it('is accepted under the policy the page itself sets', async () => {
    relay()
    const page = await worker.fetch(new Request(`${ORIGIN}/app`, { headers: { cookie: SESSION } }), env)
    expect(page.status).toBe(200)
    const res = await browserPost(page, `${ORIGIN}/app`, '/app/teams', { csrf: CSRF, name: 'Acme' })
    expect(res.status).toBe(303)
    expect(res.headers.get('location')).toContain('/app/teams/t1')
  })

  it('still refuses a cross-site form, whatever Origin it carries', async () => {
    relay()
    for (const origin of ['null', 'https://evil.example']) {
      const res = await worker.fetch(
        new Request(`${ORIGIN}/app/teams`, {
          method: 'POST',
          headers: { 'content-type': 'application/x-www-form-urlencoded', cookie: SESSION, origin, 'sec-fetch-site': 'cross-site' },
          body: new URLSearchParams({ csrf: CSRF, name: 'Acme' }).toString(),
        }),
        env,
      )
      expect(res.status, origin).toBe(403)
    }
  })

  it('accepts Origin: null only with the browser’s word that the page is this site’s, and still wants the token', async () => {
    relay()
    const post = (headers: Record<string, string>, csrf: string) =>
      worker.fetch(
        new Request(`${ORIGIN}/app/teams`, {
          method: 'POST',
          headers: { 'content-type': 'application/x-www-form-urlencoded', cookie: SESSION, origin: 'null', ...headers },
          body: new URLSearchParams({ csrf, name: 'Acme' }).toString(),
        }),
        env,
      )
    expect((await post({ 'sec-fetch-site': 'same-origin' }, CSRF)).status).toBe(303)
    expect((await post({ 'sec-fetch-site': 'same-origin' }, 'wrong')).status).toBe(403)
    expect((await post({}, CSRF)).status).toBe(403)
  })
})

describe('the refusal page', () => {
  const refused = (headers: Record<string, string>) =>
    worker.fetch(new Request(`${ORIGIN}/app/teams`, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded', cookie: SESSION, origin: 'https://evil.example', ...headers }, body: '' }), env)

  it('goes back to the page that holds the form', async () => {
    relay()
    const res = await refused({ referer: `${ORIGIN}/app/teams/team-1?x=1`, origin: ORIGIN, 'sec-fetch-site': 'same-site' })
    expect(res.status).toBe(403)
    const html = await res.text()
    expect(html).toContain('href="/app/teams/team-1"')
    expect(html).toContain('Go Back')
  })

  it('falls back to My Teams for a missing, foreign or join-link Referer', async () => {
    relay()
    for (const referer of [undefined, 'https://evil.example/app/teams/x', 'not a url', `${ORIGIN}/elsewhere`, `${ORIGIN}/app/join/${'ab'.repeat(32)}`]) {
      const res = await refused(referer ? { referer } : {})
      const html = await res.text()
      expect(html, String(referer)).toContain('href="/app">My Teams')
      expect(html).not.toContain('Go Back')
    }
  })
})

describe('a path that only takes a form post', () => {
  it.each([
    ['/app/teams', '/app'],
    ['/app/teams/team-1/invites', '/app/teams/team-1'],
    ['/app/teams/team-1/invites/inv-1/revoke', '/app/teams/team-1'],
    ['/app/teams/team-1/members/user_2/role', '/app/teams/team-1'],
  ])('sends a GET to %s on to %s, signed in or out', async (path, holder) => {
    relay()
    for (const headers of [{ cookie: SESSION }, {}]) {
      const res = await worker.fetch(new Request(`${ORIGIN}${path}`, { headers }), env)
      expect(res.status).toBe(303)
      expect(res.headers.get('location')).toBe(holder)
    }
  })

  it('leaves the paths that have a page of their own alone', async () => {
    relay()
    const res = await worker.fetch(new Request(`${ORIGIN}/app/teams/team-1/members/user_2/remove`, { headers: { cookie: SESSION } }), env)
    expect(res.status).not.toBe(303)
  })
})
