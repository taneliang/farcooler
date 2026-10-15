import { afterEach, describe, expect, it, vi } from 'vitest'

import worker, { type Env } from '../src/index'

// Sign Out Everywhere on the web (ov-564), against a relay the test plays.
// The relay's route is services/relay/src/signout.ts.

const RELAY = 'https://relay.example'
const ORIGIN = 'https://www.farcooler.com'
const ASSETS = { fetch: async () => new Response('marketing page') } as unknown as Fetcher
const env = { APPCAST_URL: 'x', ASSETS, APP_RELAY_URL: RELAY, APP_WORKOS_CLIENT_ID: 'client_TEST' } as Env

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
const jwt = (claims: Record<string, unknown>) => `${b64({ alg: 'RS256' })}.${b64(claims)}.c2lnbmF0dXJl`
const ACCESS = jwt({ sub: 'user_ME', sid: 'session_01ABC', exp: Math.floor(Date.now() / 1000) + 600 })
const CSRF = 'csrf-TEST-0123456789'
const SESSION = `__Host-fc_session=${b64({ a: ACCESS, r: 'refresh_SECRET', c: CSRF })}`

const ME = { user: { id: 'user_ME', email: 'me@example.com' }, teams: [], devices: [] }

let calls: { path: string; authorization: string | null }[] = []
function relay(answer: () => Response) {
  calls = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input))
      calls.push({ path: url.pathname, authorization: new Headers(init?.headers).get('authorization') })
      if (url.pathname === '/v1/directory/me') return Response.json(ME)
      if (url.pathname === '/v1/account/signout-all') return answer()
      return Response.json({ error: 'not found' }, { status: 404 })
    }),
  )
}
const signOutCalls = () => calls.filter(c => c.path === '/v1/account/signout-all')

const get = (path: string, cookie: string | null = SESSION) =>
  worker.fetch(new Request(`${ORIGIN}${path}`, { headers: cookie ? { cookie } : {} }), env)

function post(path: string, headers: Record<string, string> = {}, cookie: string | null = SESSION, csrf: string | null = CSRF) {
  const body = new URLSearchParams(csrf === null ? {} : { csrf })
  return worker.fetch(
    new Request(`${ORIGIN}${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded', origin: ORIGIN, ...(cookie ? { cookie } : {}), ...headers },
      body: body.toString(),
    }),
    env,
  )
}

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

const PATH = '/app/sign-out-everywhere'
const clears = (res: Response) => res.headers.getSetCookie().some(c => c.startsWith('__Host-fc_session=;') && /Max-Age=0/i.test(c))

describe('Sign Out Everywhere on the web', () => {
  it('is offered from My Devices, as a link and not a form', async () => {
    relay(() => Response.json({ ok: true, complete: true }))
    const html = await (await get('/app/devices')).text()
    expect(html).toContain(`href="${PATH}"`)
    expect(html).toContain('Sign Out Everywhere…')
  })

  it('asks first, and a GET ends nothing', async () => {
    relay(() => Response.json({ ok: true, complete: true }))
    const res = await get(PATH)
    expect(res.status).toBe(200)
    const html = await res.text()
    expect(html).toContain('Sign Out Everywhere?')
    expect(html).toContain(`action="${PATH}"`)
    expect(html).toContain(`name="csrf" value="${CSRF}"`)
    expect(html).toContain('Cancel')
    expect(signOutCalls()).toEqual([])
  })

  it('ends every session with the caller’s token, clears the cookies and says so', async () => {
    relay(() => Response.json({ ok: true, complete: true, ended: 2 }))
    const res = await post(PATH)
    expect(res.status).toBe(200)
    expect(signOutCalls()).toEqual([{ path: '/v1/account/signout-all', authorization: `Bearer ${ACCESS}` }])
    expect(clears(res)).toBe(true)
    const html = await res.text()
    expect(html).toContain('You’re signed out everywhere')
    expect(html).not.toContain('some may still be active')
  })

  it('names the runners paired lately, escaped, and says the pairings stay', async () => {
    relay(() => Response.json({ ok: true, complete: true, ended: 1, runners: [{ id: 'r1', label: 'Dev <b>Box' }, { id: 'r2', label: 'CI' }] }))
    const html = await (await post(PATH)).text()
    expect(html).toContain('Runners paired recently: Dev &lt;b&gt;Box, CI.')
    expect(html).not.toContain('<b>Box')
    relay(() => Response.json({ ok: true, complete: true, ended: 1, runners: [] }))
    expect(await (await post(PATH)).text()).not.toContain('Runners paired recently')
  })

  it('tells the person on the confirmation page to check their runners', async () => {
    relay(() => Response.json({ ok: true, complete: true }))
    expect(await (await get(PATH)).text()).toContain('Paired runners keep working. Check them for any you don’t recognize.')
  })

  it('says when the relay could not confirm every session', async () => {
    relay(() => Response.json({ ok: true, complete: false, ended: 0 }))
    const res = await post(PATH)
    expect(clears(res)).toBe(true)
    expect(await res.text()).toContain('some may still be active')
  })

  it('keeps the session and says so when the relay is down', async () => {
    relay(() => new Response('{}', { status: 500 }))
    const res = await post(PATH)
    expect(res.status).toBe(502)
    expect(clears(res)).toBe(false)
  })

  it('refuses a form from another site, and one without the token, before the relay hears of it', async () => {
    relay(() => Response.json({ ok: true, complete: true }))
    expect((await post(PATH, { origin: 'https://evil.example' })).status).toBe(403)
    expect((await post(PATH, {}, SESSION, null)).status).toBe(403)
    expect((await post(PATH, {}, SESSION, 'wrong')).status).toBe(403)
    expect(signOutCalls()).toEqual([])
  })

  it('needs a session', async () => {
    relay(() => Response.json({ ok: true, complete: true }))
    expect((await post(PATH, {}, null)).status).toBe(401)
    expect(signOutCalls()).toEqual([])
  })
})
