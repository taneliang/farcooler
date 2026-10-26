import { afterEach, describe, expect, it, vi } from 'vitest'

import { ACCESS, envWith, get, ORIGIN, relay, SESSION } from './helpers'

// Sign-in and sign-out on the client's own host. The session code is the
// site's (services/common), tested there; these check this Worker's wiring.

afterEach(() => vi.unstubAllGlobals())

const cookieValue = (res: Response, name: string) =>
  res.headers
    .getSetCookie()
    .find(c => c.startsWith(`${name}=`))
    ?.split(';')[0]
    .slice(name.length + 1)

describe('sign-in', () => {
  it('goes to AuthKit with PKCE, coming back to this host', async () => {
    const res = await get('/sign-in', envWith())
    expect(res.status).toBe(303)
    const to = new URL(res.headers.get('location')!)
    expect(to.origin + to.pathname).toBe('https://api.workos.com/user_management/authorize')
    expect(to.searchParams.get('redirect_uri')).toBe(`${ORIGIN}/callback`)
    expect(to.searchParams.get('client_id')).toBe('client_TEST')
    expect(to.searchParams.get('code_challenge_method')).toBe('S256')
    expect(res.headers.getSetCookie()[0]).toMatch(/^__Host-fc_signin=.+; Path=\/; HttpOnly; Secure; SameSite=Lax/)
  })

  it('keeps the session from a callback whose state matches, and refuses one whose state does not', async () => {
    const started = await get('/sign-in', envWith())
    const state = new URL(started.headers.get('location')!).searchParams.get('state')!
    const signin = `__Host-fc_signin=${cookieValue(started, '__Host-fc_signin')}`
    relay(undefined, { '/v1/auth/token': () => Response.json({ accessToken: ACCESS, refreshToken: 'refresh_NEW' }) })

    const wrong = await get(`/callback?code=c&state=${state}x`, envWith(), { headers: { cookie: signin } })
    expect(wrong.status).toBe(400)
    expect(wrong.headers.getSetCookie().some(c => c.startsWith('__Host-fc_session=') && !c.includes('Max-Age=0'))).toBe(false)

    const right = await get(`/callback?code=c&state=${state}`, envWith(), { headers: { cookie: signin } })
    expect(right.status).toBe(303)
    expect(right.headers.get('location')).toBe('/')
    expect(cookieValue(right, '__Host-fc_session')).toBeTruthy()
  })
})

describe('sign-out', () => {
  it('ends the session here and at WorkOS, coming back to this host', async () => {
    const res = await get('/sign-out', envWith(), { method: 'POST', headers: { cookie: SESSION, origin: ORIGIN } })
    expect(res.status).toBe(303)
    const to = new URL(res.headers.get('location')!)
    expect(to.searchParams.get('session_id')).toBe('session_01ABC')
    expect(to.searchParams.get('return_to')).toBe(`${ORIGIN}/`)
    expect(res.headers.getSetCookie()).toContain('__Host-fc_session=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0')
  })

  it("accepts the Origin: null a no-referrer form sends, only with the browser's same-origin word", async () => {
    const ok = await get('/sign-out', envWith(), { method: 'POST', headers: { cookie: SESSION, origin: 'null', 'sec-fetch-site': 'same-origin' } })
    expect(ok.status).toBe(303)
    const bare = await get('/sign-out', envWith(), { method: 'POST', headers: { cookie: SESSION, origin: 'null' } })
    expect(bare.status).toBe(403)
  })

  it('refuses a post from another site, and clears nothing', async () => {
    for (const headers of [{ origin: 'https://evil.example' }, { origin: ORIGIN, 'sec-fetch-site': 'cross-site' }]) {
      const res = await get('/sign-out', envWith(), { method: 'POST', headers: { cookie: SESSION, ...headers } })
      expect(res.status).toBe(403)
      expect(res.headers.getSetCookie()).toEqual([])
    }
  })

  it('works with the flag off, so a session made while it was on can still end', async () => {
    const res = await get('/sign-out', envWith({ WEB_CLIENT: '' }), { method: 'POST', headers: { cookie: SESSION, origin: ORIGIN } })
    expect(res.status).toBe(303)
  })
})
