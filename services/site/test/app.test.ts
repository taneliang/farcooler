import { afterEach, describe, expect, it, vi } from 'vitest'

import worker, { type Env } from '../src/index'

// The web app at /app, against a relay the test plays. The relay's shapes are
// the ones services/relay/src/directory/teams.ts returns (`me`, `listMembers`)
// and services/relay/src/index.ts's `workosToken`.

const RELAY = 'https://relay.example'
const CLIENT = 'client_TEST'
const ASSETS = { fetch: async () => new Response('marketing page') } as unknown as Fetcher
const env = { APPCAST_URL: 'x', ASSETS, APP_RELAY_URL: RELAY, APP_WORKOS_CLIENT_ID: CLIENT } as Env
const unconfigured = { APPCAST_URL: 'x', ASSETS, APP_RELAY_URL: '', APP_WORKOS_CLIENT_ID: '' } as Env

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
/// A token shaped like a WorkOS access token. The Worker never verifies it;
/// the relay (played here) does.
const jwt = (claims: Record<string, unknown>) => `${b64({ alg: 'RS256' })}.${b64(claims)}.c2lnbmF0dXJl`
const later = () => Math.floor(Date.now() / 1000) + 600
const ACCESS = jwt({ sub: 'user_ME', sid: 'session_01ABC', exp: later() })
const REFRESH = 'refresh_SECRET_0123456789'
const CSRF = 'csrf-TEST-0123456789'
const sessionFor = (access: string, refresh = REFRESH, csrf = CSRF) => `__Host-fc_session=${b64({ a: access, r: refresh, c: csrf })}`

const ME = {
  user: { id: 'user_ME', email: 'alexandra.konstantinopoulou@example.com' },
  teams: [
    { id: 'team-personal', name: 'Personal', personal: true, role: 'admin' },
    { id: 'team-1', name: 'Platform Infrastructure & <script>alert(1)</script> Reliability', personal: false, role: 'member' },
  ],
  devices: [],
}
const MEMBERS = {
  team: { id: 'team-1', name: 'Platform Infrastructure Reliability', personal: false },
  members: [
    { user: 'user_ME', email: 'alexandra.konstantinopoulou@example.com', role: 'member', joinedAt: Date.UTC(2026, 9, 11) },
    { user: 'user_2', email: 'bartholomew.oyelaran-whitfield@example.com', role: 'admin', joinedAt: Date.UTC(2026, 8, 2) },
    { user: 'user_3', email: null, role: 'billing', joinedAt: Date.UTC(2026, 9, 1) },
  ],
}

interface Call {
  path: string
  authorization: string | null
  body: any
}

/// Play the relay: `routes` maps a path to a reply, or to a function of the call.
function relay(routes: Record<string, (call: Call) => Response | Promise<Response>>): Call[] {
  const calls: Call[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input))
      expect(url.origin).toBe(RELAY)
      const call = {
        path: url.pathname,
        authorization: new Headers(init?.headers).get('authorization'),
        body: init?.body ? JSON.parse(String(init.body)) : null,
      }
      calls.push(call)
      const route = routes[url.pathname]
      if (!route) return Response.json({ error: 'not found' }, { status: 404 })
      return await route(call)
    }),
  )
  return calls
}

/// The relay as it answers a session it accepts (`ACCESS`, or `accepted`).
const signedIn = (accepted = ACCESS) => ({
  '/v1/directory/me': (call: Call) =>
    call.authorization === `Bearer ${accepted}` ? Response.json(ME) : Response.json({ error: 'unauthorized' }, { status: 401 }),
  '/v1/directory/teams/members': (call: Call) => {
    if (call.authorization !== `Bearer ${accepted}`) return Response.json({ error: 'unauthorized' }, { status: 401 })
    if (call.body?.team === 'team-1') return Response.json(MEMBERS)
    if (call.body?.team === 'team-billing') return Response.json({ error: 'forbidden' }, { status: 403 })
    return Response.json({ error: 'not found' }, { status: 404 })
  },
})

const get = (path: string, cookie?: string, environment = env) =>
  worker.fetch(new Request(`https://www.farcooler.com${path}`, { headers: cookie ? { cookie } : {} }), environment)

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('/app, not configured', () => {
  it.each(['/app', '/app/', '/app/sign-in', '/app/callback?code=x&state=y', '/app/teams/team-1'])(
    'shows "Not available yet" at %s, and calls nothing',
    async path => {
      const calls = relay(signedIn())
      const res = await get(path, sessionFor(ACCESS), unconfigured)
      expect(res.status).toBe(503)
      expect(res.headers.get('location')).toBeNull()
      expect(await res.text()).toContain('Not available yet')
      expect(calls).toHaveLength(0)
    },
  )

  it('counts a value of only spaces as unset', async () => {
    const res = await get('/app', undefined, { ...env, APP_WORKOS_CLIENT_ID: '   ' })
    expect(res.status).toBe(503)
  })
})

describe('/app, signed out', () => {
  it.each(['/app', '/app/', '/app/teams/team-1', '/app/teams/team-1/', '/app/elsewhere'])(
    'shows only sign-in at %s, and never reaches the directory',
    async path => {
      const calls = relay(signedIn())
      const res = await get(path)
      expect(res.status).toBe(200)
      const html = await res.text()
      expect(html).toContain('href="/app/sign-in"')
      expect(html).not.toContain('Platform Infrastructure')
      expect(html).not.toContain('example.com')
      expect(html).not.toContain('Sign Out')
      expect(calls).toHaveLength(0)
    },
  )

  it('clears a cookie that is not a session, without calling the relay', async () => {
    const calls = relay(signedIn())
    const res = await get('/app', '__Host-fc_session=not-a-session')
    expect(await res.text()).toContain('href="/app/sign-in"')
    expect(res.headers.get('set-cookie')).toMatch(/^__Host-fc_session=; .*Max-Age=0/)
    expect(calls).toHaveLength(0)
  })

  it('treats a session the relay refuses, and WorkOS will not refresh, as signed out', async () => {
    const forged = jwt({ sub: 'user_ME', exp: later() })
    const calls = relay({
      ...signedIn(),
      '/v1/auth/refresh': () => Response.json({ error: 'auth', status: 400 }, { status: 401 }),
    })
    for (const path of ['/app', '/app/teams/team-1']) {
      const res = await get(path, sessionFor(forged, 'refresh_forged'))
      const html = await res.text()
      expect(html).toContain('href="/app/sign-in"')
      expect(html).not.toContain('Platform Infrastructure')
      expect(html).not.toContain('example.com')
      expect(res.headers.get('set-cookie')).toMatch(/^__Host-fc_session=; .*Max-Age=0/)
    }
    expect(calls.filter(c => c.path === '/v1/auth/refresh').map(c => c.body)).toEqual([
      { refreshToken: 'refresh_forged' },
      { refreshToken: 'refresh_forged' },
    ])
  })

  it('treats a session the relay still refuses after a refresh as signed out', async () => {
    const calls = relay({
      ...signedIn('a token the relay never accepts'),
      '/v1/auth/refresh': () => Response.json({ accessToken: jwt({ exp: later() }), refreshToken: 'refresh_NEW' }),
    })
    const res = await get('/app', sessionFor(ACCESS))
    expect(await res.text()).toContain('href="/app/sign-in"')
    expect(res.headers.get('set-cookie')).toMatch(/^__Host-fc_session=; .*Max-Age=0/)
    expect(calls.map(c => c.path)).toEqual(['/v1/directory/me', '/v1/auth/refresh', '/v1/directory/me'])
  })
})

describe('/app, signed in', () => {
  it('lists my teams from the directory, with the bearer token, names escaped', async () => {
    const calls = relay(signedIn())
    const res = await get('/app', sessionFor(ACCESS))
    expect(res.status).toBe(200)
    const html = await res.text()
    expect(calls).toEqual([{ path: '/v1/directory/me', authorization: `Bearer ${ACCESS}`, body: {} }])
    expect(html).toContain('My Teams')
    expect(html).toContain('href="/app/teams/team-1"')
    expect(html).toContain('Personal team · Admin')
    expect(html).toContain('Platform Infrastructure &amp; &lt;script&gt;alert(1)&lt;/script&gt; Reliability')
    expect(html).not.toContain('<script>')
    expect(html).toContain('alexandra.konstantinopoulou@example.com')
    expect(html).toContain('Sign Out')
  })

  it("shows a team's members, marking me", async () => {
    const calls = relay(signedIn())
    const res = await get('/app/teams/team-1', sessionFor(ACCESS))
    expect(res.status).toBe(200)
    // Emails carry <wbr> break points after the @ and before each dot.
    const html = (await res.text()).replaceAll('<wbr>', '')
    expect(calls.map(c => [c.path, c.body])).toEqual([
      ['/v1/directory/me', {}],
      ['/v1/directory/teams/members', { team: 'team-1' }],
    ])
    expect(html).toContain('Platform Infrastructure Reliability')
    expect(html).toContain('3 people')
    expect(html).toContain('bartholomew.oyelaran-whitfield@example.com')
    expect(html).toContain('No email on file')
    expect(html).toContain('You · Joined Oct 11, 2026')
    expect(html).toContain('Joined Sep 2, 2026')
    expect(html.match(/class="end">(\w+)</g)).toEqual(['class="end">Member<', 'class="end">Admin<', 'class="end">Billing<'])
  })

  it("says a team isn't there, for one I'm not in, without asking for an id that isn't one", async () => {
    let calls = relay(signedIn())
    let res = await get('/app/teams/team-other', sessionFor(ACCESS))
    expect(res.status).toBe(404)
    expect(await res.text()).toContain('Team not found')

    calls = relay(signedIn())
    res = await get('/app/teams/..%2F..%2Fadmin', sessionFor(ACCESS))
    expect(res.status).toBe(404)
    expect(calls.map(c => c.path)).toEqual(['/v1/directory/me'])
  })

  it('tells a billing member the list is not theirs to see', async () => {
    relay(signedIn())
    const res = await get('/app/teams/team-billing', sessionFor(ACCESS))
    expect(res.status).toBe(403)
    const html = await res.text()
    expect(html).toContain('Members aren’t visible to you')
    expect(html).not.toContain('bartholomew')
  })

  it('refreshes an access token that is about to expire, before using it, and keeps the new pair', async () => {
    const stale = jwt({ sub: 'user_ME', exp: Math.floor(Date.now() / 1000) + 10 })
    const fresh = jwt({ sub: 'user_ME', exp: later() })
    const calls = relay({
      ...signedIn(fresh),
      '/v1/auth/refresh': () => Response.json({ accessToken: fresh, refreshToken: 'refresh_NEW', userId: 'user_ME', email: '' }),
    })
    const res = await get('/app', sessionFor(stale))
    expect(res.status).toBe(200)
    expect(await res.text()).toContain('My Teams')
    expect(calls.map(c => c.path)).toEqual(['/v1/auth/refresh', '/v1/directory/me'])
    const set = res.headers.get('set-cookie') ?? ''
    expect(set).toBe(`${sessionFor(fresh, 'refresh_NEW')}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=2592000`)
  })

  it('refreshes once when the relay refuses the access token, then retries', async () => {
    const fresh = jwt({ sub: 'user_ME', exp: later() })
    const calls = relay({
      ...signedIn(fresh),
      '/v1/auth/refresh': () => Response.json({ accessToken: fresh, refreshToken: 'refresh_NEW' }),
    })
    const res = await get('/app', sessionFor(ACCESS))
    expect(await res.text()).toContain('My Teams')
    expect(calls.map(c => c.path)).toEqual(['/v1/directory/me', '/v1/auth/refresh', '/v1/directory/me'])
  })

  it('keeps the session, and offers Try Again, when the relay is down', async () => {
    relay({ '/v1/directory/me': () => new Response('oops', { status: 500 }) })
    const res = await get('/app', sessionFor(ACCESS))
    expect(res.status).toBe(502)
    expect(await res.text()).toContain('Try Again')
    expect(res.headers.get('set-cookie')).toBeNull()
  })
})

describe('signing in and out', () => {
  it('sends the browser to AuthKit with PKCE, and remembers the state and verifier for the callback', async () => {
    relay({})
    const res = await get('/app/sign-in')
    expect(res.status).toBe(303)
    const to = new URL(res.headers.get('location')!)
    expect(to.origin + to.pathname).toBe('https://api.workos.com/user_management/authorize')
    expect(to.searchParams.get('client_id')).toBe(CLIENT)
    expect(to.searchParams.get('redirect_uri')).toBe('https://www.farcooler.com/app/callback')
    expect(to.searchParams.get('provider')).toBe('authkit')
    expect(to.searchParams.get('code_challenge_method')).toBe('S256')

    const set = res.headers.get('set-cookie')!
    expect(set).toMatch(/Path=\/; HttpOnly; Secure; SameSite=Lax; Max-Age=600$/)
    const saved = JSON.parse(atob(set.split(';')[0].split('=')[1].replaceAll('-', '+').replaceAll('_', '/')))
    expect(saved.s).toBe(to.searchParams.get('state'))
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(saved.v)))
    const challenge = btoa(String.fromCharCode(...digest)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
    expect(to.searchParams.get('code_challenge')).toBe(challenge)
  })

  const pending = `__Host-fc_signin=${b64({ s: 'state-123', v: 'verifier-456' })}`

  it('trades the code through the relay, proving the verifier, and keeps the session', async () => {
    const calls = relay({
      '/v1/auth/token': () => Response.json({ accessToken: ACCESS, refreshToken: REFRESH, userId: 'user_ME', email: '' }),
    })
    const res = await get('/app/callback?code=code-789&state=state-123', pending)
    expect(res.status).toBe(303)
    expect(res.headers.get('location')).toBe('/app')
    expect(calls).toEqual([{ path: '/v1/auth/token', authorization: null, body: { code: 'code-789', verifier: 'verifier-456' } }])
    const set = res.headers.getSetCookie()
    // The session carries a CSRF token minted here, and nothing else new.
    const session = set.find(c => c.startsWith('__Host-fc_session='))!
    const saved = JSON.parse(atob(session.split(';')[0].split('=')[1].replaceAll('-', '+').replaceAll('_', '/')))
    expect(saved).toEqual({ a: ACCESS, r: REFRESH, c: expect.stringMatching(/^[A-Za-z0-9_-]{40,}$/) })
    expect(session).toMatch(/; Path=\/; HttpOnly; Secure; SameSite=Lax; Max-Age=2592000$/)
    expect(set.some(c => c.startsWith('__Host-fc_signin=; '))).toBe(true)
  })

  it.each([
    ['a state that is not this browser’s', '/app/callback?code=c&state=someone-else', pending],
    ['no sign-in in flight', '/app/callback?code=c&state=state-123', undefined],
    ['no code', '/app/callback?state=state-123', pending],
    ['an error from AuthKit', '/app/callback?error=access_denied&state=state-123', pending],
  ])('refuses %s, without trading anything', async (_what, path, cookie) => {
    const calls = relay({ '/v1/auth/token': () => Response.json({ accessToken: ACCESS, refreshToken: REFRESH }) })
    const res = await get(path, cookie)
    expect(res.status).toBe(400)
    expect(await res.text()).toContain('Sign-in didn’t finish')
    expect(res.headers.getSetCookie().some(c => c.startsWith('__Host-fc_session=__'))).toBe(false)
    expect(calls).toHaveLength(0)
  })

  it('keeps no session when the relay refuses the code', async () => {
    relay({ '/v1/auth/token': () => Response.json({ error: 'auth', status: 400 }, { status: 401 }) })
    const res = await get('/app/callback?code=c&state=state-123', pending)
    expect(res.status).toBe(400)
    expect(res.headers.getSetCookie().some(c => c.startsWith('__Host-fc_session='))).toBe(false)
  })

  it('signs out by POST only, from this site only, with the session’s token, and ends the WorkOS session too', async () => {
    relay({})
    const post = (headers: Record<string, string>, csrf: string | null = CSRF) =>
      worker.fetch(
        new Request('https://www.farcooler.com/app/sign-out', {
          method: 'POST',
          headers: { cookie: sessionFor(ACCESS), 'content-type': 'application/x-www-form-urlencoded', ...headers },
          body: csrf === null ? '' : `csrf=${csrf}`,
        }),
        env,
      )
    expect((await get('/app/sign-out', sessionFor(ACCESS))).status).toBe(405)
    expect((await post({ origin: 'https://evil.example' })).status).toBe(403)
    expect((await post({ origin: 'https://www.farcooler.com' }, null)).status).toBe(403)
    expect((await post({ origin: 'https://www.farcooler.com' }, 'someone-elses')).status).toBe(403)

    const res = await post({ origin: 'https://www.farcooler.com' })
    expect(res.status).toBe(303)
    const to = new URL(res.headers.get('location')!)
    expect(to.origin + to.pathname).toBe('https://api.workos.com/user_management/sessions/logout')
    expect(to.searchParams.get('session_id')).toBe('session_01ABC')
    expect(to.searchParams.get('return_to')).toBe('https://www.farcooler.com/app')
    expect(res.headers.get('set-cookie')).toMatch(/^__Host-fc_session=; .*Max-Age=0/)
  })
})

describe('the app and the rest of the site', () => {
  it('sends every app page with no-store and a policy that allows no script', async () => {
    relay(signedIn())
    for (const res of [await get('/app'), await get('/app', sessionFor(ACCESS)), await get('/app', undefined, unconfigured)]) {
      expect(res.headers.get('cache-control')).toBe('no-store')
      const csp = res.headers.get('content-security-policy') ?? ''
      expect(csp).toContain("default-src 'none'")
      expect(csp).not.toContain('script-src')
      expect(res.headers.get('referrer-policy')).toBe('no-referrer')
    }
  })

  it.each(['/', '/apple-touch-icon.png', '/application', '/download'])('leaves %s to the marketing site', async path => {
    const res = await get(path, sessionFor(ACCESS))
    expect(await res.text()).toBe('marketing page')
  })
})
