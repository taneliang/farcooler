import { afterEach, describe, expect, it, vi } from 'vitest'

import worker, { type Env } from '../src/index'

// Managing a team and joining one from the web (ov-524), against a relay the
// test plays. The relay's shapes are services/relay/src/directory/{teams,
// invites}.ts's. The security guards (same origin, the CSRF token, the code
// kept out of URLs) each have a test here that fails without the guard.

const RELAY = 'https://relay.example'
const ORIGIN = 'https://www.farcooler.com'
const ASSETS = { fetch: async () => new Response('marketing page') } as unknown as Fetcher
const env = { APPCAST_URL: 'x', ASSETS, APP_RELAY_URL: RELAY, APP_WORKOS_CLIENT_ID: 'client_TEST' } as Env

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
const jwt = (claims: Record<string, unknown>) => `${b64({ alg: 'RS256' })}.${b64(claims)}.c2lnbmF0dXJl`
const ACCESS = jwt({ sub: 'user_ME', sid: 'session_01ABC', exp: Math.floor(Date.now() / 1000) + 600 })
const CSRF = 'csrf-TEST-0123456789'
const SESSION = `__Host-fc_session=${b64({ a: ACCESS, r: 'refresh_SECRET', c: CSRF })}`
const CODE = 'ab12'.repeat(16)
const JOIN = `__Host-fc_signin=${b64({ j: CODE })}`

const NOW = Date.now()
const DAY = 24 * 60 * 60 * 1000
const person = (n: number, role = 'member') => ({
  user: `user_${n}`,
  email: `teammate.with-a-long-name-${n}@a-company-with-a-long-name.example.com`,
  role,
  joinedAt: Date.UTC(2026, 8, 1 + (n % 28)),
})
const MEMBERS = {
  team: { id: 'team-1', name: 'Platform Infrastructure', personal: false },
  members: [{ ...person(0, 'admin'), user: 'user_ME', email: 'me@example.com' }, person(1, 'admin'), person(2), person(3, 'billing')],
  removed: [{ user: 'user_9', email: 'gone@example.com', removedAt: Date.UTC(2026, 9, 1) }],
}
const invite = (id: string, over: Record<string, unknown> = {}) => ({
  id,
  role: 'member',
  domain: null,
  maxUses: 50,
  uses: 3,
  createdBy: 'user_ME',
  createdAt: NOW - DAY,
  expiresAt: NOW + 6 * DAY,
  revokedAt: null,
  ...over,
})
const INVITES = {
  invites: [
    invite('inv-open'),
    invite('inv-domain', { domain: 'acme.com', role: 'billing' }),
    invite('inv-revoked', { revokedAt: NOW - 1000 }),
    invite('inv-expired', { expiresAt: NOW - 1000 }),
    invite('inv-used', { uses: 50 }),
  ],
}

interface Call {
  path: string
  authorization: string | null
  body: any
}
type Routes = Record<string, (call: Call) => Response | Promise<Response>>

function relay(routes: Routes): Call[] {
  const calls: Call[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input))
      const call = {
        path: url.pathname.replace('/v1/directory/', ''),
        authorization: new Headers(init?.headers).get('authorization'),
        body: init?.body ? JSON.parse(String(init.body)) : null,
      }
      calls.push(call)
      const route = routes[call.path]
      return route ? await route(call) : Response.json({ error: 'not found' }, { status: 404 })
    }),
  )
  return calls
}

const ME = (role = 'admin') => ({
  user: { id: 'user_ME', email: 'me@example.com' },
  teams: [
    { id: 'personal-1', name: 'Personal', personal: true, role: 'admin' },
    { id: 'team-1', name: 'Platform Infrastructure', personal: false, role },
  ],
  devices: [],
})
const asAdmin = (over: Routes = {}, role = 'admin'): Routes => ({
  me: () => Response.json(ME(role)),
  'teams/members': () => Response.json(MEMBERS),
  'teams/invites/list': () => Response.json(INVITES),
  'teams/runners': () => Response.json({ runners: [] }),
  ...over,
})
const ok = () => Response.json({ ok: true })

const get = (path: string, cookie = SESSION) => worker.fetch(new Request(`${ORIGIN}${path}`, { headers: cookie ? { cookie } : {} }), env)

function post(path: string, fields: Record<string, string> = {}, headers: Record<string, string | undefined> = {}, cookie: string | null = SESSION, csrf: string | null = CSRF) {
  const body = new URLSearchParams({ ...(csrf === null ? {} : { csrf }), ...fields })
  return worker.fetch(
    new Request(`${ORIGIN}${path}`, {
      method: 'POST',
      headers: Object.fromEntries(
        Object.entries({ 'content-type': 'application/x-www-form-urlencoded', origin: ORIGIN, ...(cookie ? { cookie } : {}), ...headers }).filter(([, v]) => v !== undefined),
      ),
      body: body.toString(),
    }),
    env,
  )
}

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

const text = async (res: Response) => (await res.text()).replaceAll('<wbr>', '')
const forms = (html: string) => [...html.matchAll(/<form[^>]*action="([^"]+)"/g)].map(m => m[1])

describe('the team page', () => {
  it('gives an admin an invite form with the ruled defaults, the open links, the people and the removed', async () => {
    relay(asAdmin())
    const res = await get('/app/teams/team-1')
    expect(res.status).toBe(200)
    const html = await text(res)
    // R-63: member role, 7 days, 50 uses, no domain.
    expect(html).toContain('<option value="member" selected>')
    expect(html).toContain('<option value="7" selected>')
    expect(html).toContain('name="uses" value="50"')
    expect(html).toMatch(/name="domain" placeholder/)
    expect(html).not.toMatch(/name="domain"[^>]*value=/)
    expect(html).toContain('New Invite Link')
    // Open links only, each with a way to revoke it.
    expect(html).toContain('47 uses left')
    expect(html).toContain('Only @acme.com')
    expect(html).toContain('/app/teams/team-1/invites/inv-open/revoke')
    expect(html).toContain('/app/teams/team-1/invites/inv-domain/revoke')
    for (const gone of ['inv-revoked', 'inv-expired', 'inv-used']) expect(html).not.toContain(gone)
    // Everyone is a way into their page.
    expect(html).toContain('href="/app/teams/team-1/members/user_2"')
    // The removed can be readmitted.
    expect(html).toContain('gone@example.com')
    expect(html).toContain('/app/teams/team-1/members/user_9/readmit')
    expect(html).toContain('Readmit')
    // Every form carries the session's token.
    expect(html.match(/name="csrf" value="csrf-TEST-0123456789"/g)!.length).toBe(forms(html).length)
  })

  it('shows a member the list and nothing to change', async () => {
    const calls = relay(asAdmin({}, 'member'))
    const html = await text(await get('/app/teams/team-1'))
    expect(html).toContain('teammate.with-a-long-name-1@a-company-with-a-long-name.example.com')
    expect(html).not.toContain('New Invite Link')
    expect(html).not.toContain('/members/user_2')
    expect(html).not.toContain('Readmit')
    expect(html).not.toContain('Revoke')
    // Only sign out is a form, and nothing asked for invites or the removed.
    expect(forms(html)).toEqual(['/app/sign-out'])
    expect(calls.map(c => c.path)).toEqual(['me', 'teams/members'])
  })

  it('gives a personal team no controls, even to its admin', async () => {
    relay(asAdmin({ 'teams/members': () => Response.json({ team: { id: 'personal-1', name: 'Personal', personal: true }, members: [MEMBERS.members[0]] }) }))
    const res = await get('/app/teams/personal-1')
    const html = await text(res)
    expect(html).toContain('Your personal team')
    expect(html).not.toContain('New Invite Link')
    expect(forms(html)).toEqual(['/app/sign-out'])
    expect((await get('/app/teams/personal-1/members/user_ME')).status).toBe(303)
  })

  it('lists 50 people with long names, and says how many', async () => {
    const many = { ...MEMBERS, members: [MEMBERS.members[0], ...Array.from({ length: 49 }, (_, i) => person(i + 10))] }
    relay(asAdmin({ 'teams/members': () => Response.json(many) }))
    const html = await text(await get('/app/teams/team-1'))
    expect(html).toContain('50 people')
    expect(html.match(/class="avatar"/g)!.length).toBeGreaterThanOrEqual(50)
  })

  it('shows a notice from a fixed set, never the query’s own words', async () => {
    relay(asAdmin())
    expect(await text(await get('/app/teams/team-1?notice=role-changed'))).toContain('Role changed.')
    const html = await text(await get('/app/teams/team-1?notice=%3Cb%3Ehello%3C/b%3E'))
    expect(html).not.toContain('hello')
  })
})

describe('the guards on every POST', () => {
  const POSTS: [string, string, Record<string, string>][] = [
    ['sign out', '/app/sign-out', {}],
    ['create a team', '/app/teams', { name: 'Acme' }],
    ['create an invite', '/app/teams/team-1/invites', { role: 'member', days: '7', uses: '50' }],
    ['revoke an invite', '/app/teams/team-1/invites/inv-open/revoke', {}],
    ['change a role', '/app/teams/team-1/members/user_2/role', { role: 'admin' }],
    ['remove a member', '/app/teams/team-1/members/user_2/remove', {}],
    ['readmit a member', '/app/teams/team-1/members/user_9/readmit', {}],
    ['join', '/app/join', {}],
  ]

  describe.each(POSTS)('%s', (_name, path, fields) => {
    it('is refused without a same-origin Origin or Sec-Fetch-Site, and asks the relay nothing', async () => {
      const calls = relay(asAdmin({ 'teams/members/role': ok, 'teams/members/remove': ok, 'teams/members/readmit': ok, 'teams/invites/revoke': ok }))
      const refusals: Record<string, string | undefined>[] = [
        { origin: 'https://evil.example' },
        { origin: 'null' },
        { origin: `${ORIGIN}.evil.example` },
        { origin: 'http://www.farcooler.com' },
        // No Origin, and no word that it is the same site.
        { origin: undefined },
        { origin: undefined, 'sec-fetch-site': 'cross-site' },
        { origin: undefined, 'sec-fetch-site': 'same-site' },
        { origin: undefined, 'sec-fetch-site': 'none' },
        // Origin says ours, the browser says otherwise.
        { origin: ORIGIN, 'sec-fetch-site': 'cross-site' },
      ]
      for (const headers of refusals) {
        const res = await post(path, fields, headers)
        expect(res.status, JSON.stringify(headers)).toBe(403)
        expect(res.headers.get('location')).toBeNull()
      }
      expect(calls).toHaveLength(0)
    })

    it('accepts Sec-Fetch-Site: same-origin when a browser sends no Origin', async () => {
      relay(asAdmin({ 'teams/create': () => Response.json({ team: { id: 't' } }), 'teams/invites/create': () => Response.json({ code: CODE, invite: 'i', expiresAt: NOW }) }))
      const res = await post(path, fields, { origin: undefined, 'sec-fetch-site': 'same-origin' })
      expect(res.status).not.toBe(403)
    })

    it('is refused without this session’s token, and asks the relay nothing', async () => {
      const calls = relay(asAdmin({ 'teams/members/role': ok, 'teams/members/remove': ok, 'teams/members/readmit': ok, 'teams/invites/revoke': ok }))
      for (const csrf of [null, '', 'another-sessions-token', CSRF.slice(0, -1), `${CSRF}x`]) {
        const res = await post(path, fields, {}, SESSION, csrf)
        expect(res.status, String(csrf)).toBe(403)
      }
      // A session from before tokens existed has none to match.
      const old = `__Host-fc_session=${b64({ a: ACCESS, r: 'refresh_SECRET' })}`
      expect((await post(path, fields, {}, old, CSRF)).status).toBe(403)
      expect(calls).toHaveLength(0)
    })
  })

  it('is signed out for a POST with no session, and does nothing', async () => {
    const calls = relay(asAdmin())
    const res = await post('/app/teams/team-1/members/user_2/remove', {}, {}, null)
    expect(res.status).toBe(401)
    expect(await res.text()).toContain('href="/app/sign-in"')
    expect(calls).toHaveLength(0)
  })

  it('refuses a GET to a path that only takes POST, and a POST to a page that does not exist', async () => {
    relay(asAdmin())
    expect((await get('/app/sign-out')).status).toBe(405)
    expect((await post('/app/nowhere', {})).status).toBe(405)
  })

  it('gives a session from before tokens existed one, with the page', async () => {
    relay(asAdmin())
    const old = `__Host-fc_session=${b64({ a: ACCESS, r: 'refresh_SECRET' })}`
    const res = await get('/app/teams/team-1', old)
    const set = res.headers.get('set-cookie')!
    const saved = JSON.parse(atob(set.split(';')[0].split('=')[1].replaceAll('-', '+').replaceAll('_', '/')))
    expect(saved.c).toMatch(/^[A-Za-z0-9_-]{40,}$/)
    expect(await text(res)).toContain(`name="csrf" value="${saved.c}"`)
  })
})

describe('changes an admin makes', () => {
  it('creates an invite with the defaults, and shows the link once with its expiry and use limit', async () => {
    const expiresAt = NOW + 7 * DAY
    const calls = relay(asAdmin({ 'teams/invites/create': () => Response.json({ invite: 'inv-new', code: CODE, expiresAt }) }))
    const res = await post('/app/teams/team-1/invites', { role: 'member', days: '7', uses: '50', domain: '  ' })
    expect(res.status).toBe(200)
    expect(calls.find(c => c.path === 'teams/invites/create')!.body).toEqual({ team: 'team-1', role: 'member', days: 7, uses: 50 })
    const html = await res.text()
    expect(html).toContain(`${ORIGIN}/app/join/${CODE}`)
    expect(html).toContain('Copy it now: it won’t be shown again.')
    expect(html).toContain('for 50 people')
    expect(html).toMatch(/works until [A-Z][a-z]{2} \d{1,2}, \d{4}/)
    expect(res.headers.get('cache-control')).toBe('no-store')
    expect(res.headers.get('referrer-policy')).toBe('no-referrer')
    // Never in a header, so not in a redirect either.
    expect([...res.headers].map(([n, v]) => `${n}: ${v}`).join('\n')).not.toContain(CODE)
  })

  it('sends a domain when one is set, lower-cased by the relay, and a billing role', async () => {
    const calls = relay(asAdmin({ 'teams/invites/create': () => Response.json({ invite: 'i', code: CODE, expiresAt: NOW + DAY }) }))
    const res = await post('/app/teams/team-1/invites', { role: 'billing', days: '1', uses: '1', domain: 'Acme.com' })
    expect(calls.find(c => c.path === 'teams/invites/create')!.body).toEqual({ team: 'team-1', role: 'billing', days: 1, uses: 1, domain: 'Acme.com' })
    const html = await res.text()
    expect(html).toContain('as a billing member')
    expect(html).toContain('for 1 person')
    expect(html).toContain('only for @acme.com email addresses')
  })

  it.each([
    ['name', 'err-name', '/app'],
    ['domain', 'err-domain', '/app/teams/team-1'],
    ['days', 'err-days', '/app/teams/team-1'],
    ['uses', 'err-uses', '/app/teams/team-1'],
  ])('says what is wrong when the relay refuses a %s', async (field, notice, back) => {
    relay(asAdmin({ 'teams/create': () => Response.json({ error: field }, { status: 400 }), 'teams/invites/create': () => Response.json({ error: field }, { status: 400 }) }))
    const res =
      field === 'name' ? await post('/app/teams', { name: '' }) : await post('/app/teams/team-1/invites', { role: 'member', days: '7', uses: '50' })
    expect(res.status).toBe(303)
    expect(res.headers.get('location')).toBe(`${back}?notice=${notice}`)
  })

  it('creates a team and opens it', async () => {
    const calls = relay({ 'teams/create': () => Response.json({ team: { id: 'team-new', name: 'Acme', personal: false, role: 'admin' } }) })
    const res = await post('/app/teams', { name: 'Acme' })
    expect(calls[0].body).toEqual({ name: 'Acme' })
    expect(res.status).toBe(303)
    expect(res.headers.get('location')).toBe('/app/teams/team-new?notice=team-created')
  })

  it.each([
    ['revokes an invite', '/app/teams/team-1/invites/inv-open/revoke', {}, 'teams/invites/revoke', { team: 'team-1', invite: 'inv-open' }, 'revoked'],
    ['changes a role', '/app/teams/team-1/members/user_2/role', { role: 'billing' }, 'teams/members/role', { team: 'team-1', user: 'user_2', role: 'billing' }, 'role-changed'],
    ['removes a member', '/app/teams/team-1/members/user_2/remove', {}, 'teams/members/remove', { team: 'team-1', user: 'user_2' }, 'removed'],
    ['readmits a member', '/app/teams/team-1/members/user_9/readmit', {}, 'teams/members/readmit', { team: 'team-1', user: 'user_9' }, 'readmitted'],
  ])('%s, and goes back to the team page', async (_name, path, fields, route, body, notice) => {
    const calls = relay(asAdmin({ [route]: ok }))
    const res = await post(path, fields)
    expect(calls.map(c => [c.path, c.body])).toEqual([[route, body]])
    expect(calls[0].authorization).toBe(`Bearer ${ACCESS}`)
    expect(res.status).toBe(303)
    expect(res.headers.get('location')).toBe(`/app/teams/team-1?notice=${notice}`)
  })

  it('says the last admin cannot be demoted or removed, as a sentence', async () => {
    relay(asAdmin({ 'teams/members/role': () => Response.json({ error: 'last admin' }, { status: 409 }), 'teams/members/remove': () => Response.json({ error: 'last admin' }, { status: 409 }) }))
    for (const [path, fields] of [
      ['/app/teams/team-1/members/user_1/role', { role: 'member' }],
      ['/app/teams/team-1/members/user_1/remove', {}],
    ] as const) {
      const res = await post(path, fields)
      expect(res.headers.get('location')).toBe('/app/teams/team-1?notice=err-last-admin')
    }
    expect(await text(await get('/app/teams/team-1?notice=err-last-admin'))).toContain('A team needs at least one admin. Make someone else an admin first.')
  })

  it('says a personal team has no people to manage', async () => {
    relay(asAdmin({ 'teams/members/role': () => Response.json({ error: 'personal' }, { status: 409 }) }))
    const res = await post('/app/teams/team-1/members/user_2/role', { role: 'admin' })
    expect(res.headers.get('location')).toBe('/app/teams/team-1?notice=err-personal')
  })

  it('lands on the team page, saying the person is gone, when the relay cannot find them', async () => {
    relay(asAdmin({ 'teams/members/remove': () => Response.json({ error: 'not found' }, { status: 404 }) }))
    const res = await post('/app/teams/team-1/members/user_7/remove')
    expect(res.headers.get('location')).toBe('/app/teams/team-1?notice=err-gone')
  })

  it('gives a member who forges any of these POSTs the relay’s 403, and changes nothing', async () => {
    const forbidden = () => Response.json({ error: 'forbidden' }, { status: 403 })
    const calls = relay(
      asAdmin(
        {
          'teams/invites/create': forbidden,
          'teams/invites/revoke': forbidden,
          'teams/members/role': forbidden,
          'teams/members/remove': forbidden,
          'teams/members/readmit': forbidden,
        },
        'member',
      ),
    )
    for (const [path, fields] of [
      ['/app/teams/team-1/invites', { role: 'member', days: '7', uses: '50' }],
      ['/app/teams/team-1/invites/inv-open/revoke', {}],
      ['/app/teams/team-1/members/user_2/role', { role: 'admin' }],
      ['/app/teams/team-1/members/user_2/remove', {}],
      ['/app/teams/team-1/members/user_9/readmit', {}],
    ] as const) {
      const res = await post(path, fields)
      expect(res.status, path).toBe(403)
      const html = await res.text()
      expect(html).toContain('Only admins can do that')
      expect(html).not.toContain(CODE)
    }
    // The relay was asked each time (and said no): the web does not decide.
    expect(calls.filter(c => c.path.startsWith('teams/') && c.path !== 'teams/members').length).toBe(5)
  })

  it('does not ask a member for the controls pages either', async () => {
    relay(asAdmin({}, 'member'))
    for (const path of ['/app/teams/team-1/members/user_2', '/app/teams/team-1/members/user_2/remove', '/app/teams/team-1/members/user_2/role/admin']) {
      expect((await get(path)).status, path).toBe(403)
    }
  })
})

describe('the confirm steps', () => {
  it('shows a member’s page with their role choices, and nothing for the only admin', async () => {
    relay(asAdmin())
    const html = await text(await get('/app/teams/team-1/members/user_2'))
    expect(html).toContain('Make Admin')
    expect(html).toContain('Make Billing')
    expect(html).not.toContain('Make Member')
    expect(html).toContain('href="/app/teams/team-1/members/user_2/remove"')
    // Choosing only opens a confirm page: nothing here posts a change.
    expect(forms(html)).toEqual(['/app/sign-out'])

    const alone = { ...MEMBERS, members: [MEMBERS.members[0], person(2)] }
    relay(asAdmin({ 'teams/members': () => Response.json(alone) }))
    const mine = await text(await get('/app/teams/team-1/members/user_ME'))
    expect(mine).toContain('the only admin')
    expect(mine).not.toContain('Make Member')
    expect((await get('/app/teams/team-1/members/user_ME/role/member')).status).toBe(303)
  })

  it('asks before a role changes, and says what the new role can do', async () => {
    relay(asAdmin())
    const up = await text(await get('/app/teams/team-1/members/user_2/role/admin'))
    expect(up).toContain('Make Admin?')
    expect(up).toContain('They’ll be able to manage people, roles and invite links.')
    expect(forms(up)).toContain('/app/teams/team-1/members/user_2/role')
    expect(up).toContain('name="role" value="admin"')
    const down = await text(await get('/app/teams/team-1/members/user_1/role/billing'))
    expect(down).toContain('no longer be able to manage people')
    // Nothing changed by asking.
    expect(await text(await get('/app/teams/team-1/members/user_2/role/member'))).not.toContain('Make Member?')
  })

  it('says a removal leaves the keys on runners, and names the runners where they hold a role', async () => {
    const calls = relay(
      asAdmin({
        'teams/runners': () =>
          Response.json({
            runners: [
              { id: 'r1', name: 'build-box', access: [{ user: 'user_2', email: 'x', role: 'control' }, { user: 'user_1', email: 'y', role: 'owner' }] },
              { id: 'r2', name: 'mac-mini <b>', access: [{ user: 'user_2', email: 'x', role: 'read' }] },
              { id: 'r3', name: 'not-theirs', access: [{ user: 'user_1', email: 'y', role: 'owner' }] },
            ],
          }),
      }),
    )
    const html = await text(await get('/app/teams/team-1/members/user_2/remove'))
    expect(html).toContain('Remove from Team?')
    expect(html).toContain('Their keys stay on runners until a runner admin revokes them.')
    expect(html).toContain('build-box')
    expect(html).toContain('mac-mini &lt;b&gt;')
    expect(html).not.toContain('not-theirs')
    expect(html).toContain('Control')
    expect(forms(html)).toContain('/app/teams/team-1/members/user_2/remove')
    expect(calls.map(c => c.path)).toContain('teams/runners')
  })

  it('says so when they hold a role on no runner, and still that the keys stay', async () => {
    relay(asAdmin())
    const html = await text(await get('/app/teams/team-1/members/user_2/remove'))
    expect(html).toContain('Their keys stay on runners until a runner admin revokes them.')
    expect(html).toContain('They don’t hold a role on any of this team’s runners.')
  })

  it('does not claim they hold no role when the runner list could not load', async () => {
    relay(asAdmin({ 'teams/runners': () => new Response('down', { status: 500 }) }))
    const res = await get('/app/teams/team-1/members/user_2/remove')
    expect(res.status).toBe(502)
    expect(await res.text()).not.toContain('don’t hold a role')
  })

  it('does not offer to remove yourself, or someone not in the team', async () => {
    relay(asAdmin())
    expect((await get('/app/teams/team-1/members/user_ME/remove')).headers.get('location')).toBe('/app/teams/team-1/members/user_ME')
    expect((await get('/app/teams/team-1/members/user_404')).headers.get('location')).toBe('/app/teams/team-1?notice=err-gone')
  })
})

describe('joining from a link', () => {
  const asVisitor = (over: Routes = {}): Routes => ({
    me: () => Response.json(ME()),
    'invites/preview': () => Response.json({ team: { id: 'team-1', name: 'Platform Infrastructure' }, role: 'member', member: false }),
    'invites/accept': () => Response.json({ team: { id: 'team-1', name: 'Platform Infrastructure' }, role: 'member', joined: true }),
    ...over,
  })

  it('moves the code from the path into the HttpOnly cookie at once, whoever opens it', async () => {
    const calls = relay(asVisitor())
    for (const cookie of ['', SESSION]) {
      const res = await get(`/app/join/${CODE}`, cookie)
      expect(res.status).toBe(303)
      expect(res.headers.get('location')).toBe('/app/join')
      expect(res.headers.get('location')).not.toContain(CODE)
      expect(res.headers.get('set-cookie')).toMatch(/^__Host-fc_signin=[^;]+; Path=\/; HttpOnly; Secure; SameSite=Lax; Max-Age=600$/)
      expect(await res.text()).toBe('')
    }
    expect(calls).toHaveLength(0)
  })

  it('takes the code through sign-in in the cookie, and never in a query string', async () => {
    relay(asVisitor({ '../auth/token': () => Response.json({}) }))
    // Signed out at the confirmation: a page that says why, and Sign In.
    const waiting = await get('/app/join', JOIN)
    const html = await waiting.text()
    expect(html).toContain('Join a team on Far Cooler')
    expect(html).toContain('href="/app/sign-in"')
    expect(html).not.toContain(CODE)

    // Sign In keeps the code in the sign-in cookie, with the state and verifier.
    const start = await get('/app/sign-in', JOIN)
    const authorize = start.headers.get('location')!
    expect(authorize).not.toContain(CODE)
    expect(new URL(authorize).searchParams.has('code')).toBe(false)
    const cookie = start.headers.get('set-cookie')!
    const saved = JSON.parse(atob(cookie.split(';')[0].split('=')[1].replaceAll('-', '+').replaceAll('_', '/')))
    expect(saved.j).toBe(CODE)
    expect(saved.s).toBe(new URL(authorize).searchParams.get('state'))

    // Back from AuthKit: the session, and on to the confirmation.
    relay({ '../auth/token': () => Response.json({}) })
    vi.stubGlobal('fetch', vi.fn(async () => Response.json({ accessToken: ACCESS, refreshToken: 'refresh_SECRET' })))
    const back = await get(`/app/callback?code=authkit-code&state=${saved.s}`, cookie.split(';')[0])
    expect(back.status).toBe(303)
    expect(back.headers.get('location')).toBe('/app/join')
    expect(back.headers.get('location')).not.toContain(CODE)
    const set = back.headers.getSetCookie()
    expect(set.some(c => c.startsWith('__Host-fc_session='))).toBe(true)
    expect(set.find(c => c.startsWith('__Host-fc_signin='))).toContain('HttpOnly')
  })

  it('asks "Join Platform Infrastructure?" with the code kept off the page, and joins on the button', async () => {
    const calls = relay(asVisitor())
    const page = await get('/app/join', `${SESSION}; ${JOIN}`)
    expect(page.status).toBe(200)
    const html = await text(page)
    expect(html).toContain('Join Platform Infrastructure?')
    expect(html).toContain('as a member')
    expect(html).toContain('>Join<')
    expect(html).not.toContain(CODE)
    expect(page.headers.get('referrer-policy')).toBe('no-referrer')
    // Asking spent nothing: only a preview.
    expect(calls.map(c => c.path)).toEqual(['me', 'invites/preview'])
    expect(calls[1].body).toEqual({ code: CODE })

    const joined = await post('/app/join', {}, {}, `${SESSION}; ${JOIN}`)
    expect(calls.at(-1)).toMatchObject({ path: 'invites/accept', body: { code: CODE } })
    expect(joined.status).toBe(303)
    expect(joined.headers.get('location')).toBe('/app/teams/team-1?notice=joined')
    expect(joined.headers.get('location')).not.toContain(CODE)
    expect(joined.headers.getSetCookie().some(c => /^__Host-fc_signin=; .*Max-Age=0/.test(c))).toBe(true)
  })

  it('tells someone already in the team, and spends nothing', async () => {
    relay(asVisitor({ 'invites/preview': () => Response.json({ team: { id: 'team-1', name: 'Platform Infrastructure' }, role: 'member', member: true }) }))
    const html = await text(await get('/app/join', `${SESSION}; ${JOIN}`))
    expect(html).toContain('You’re already in Platform Infrastructure')
    expect(html).toContain('Open Team')
  })

  const FAILURES: [reason: string, status: number, title: string][] = [
    ['used', 404, 'This invite is used up'],
    ['expired', 404, 'This invite has expired'],
    ['revoked', 404, 'This invite was revoked'],
    ['removed', 404, 'You were removed from this team'],
    ['unknown', 404, 'This invite isn’t valid'],
    ['domain', 403, 'This invite is for another email'],
  ]

  it.each(FAILURES)('says plainly when a link is %s, before joining and when joining', async (reason, status, title) => {
    const refusal = () => (reason === 'domain' ? Response.json({ error: 'domain' }, { status }) : Response.json({ error: 'not found', reason }, { status }))
    relay(asVisitor({ 'invites/preview': refusal, 'invites/accept': refusal }))
    const before = await get('/app/join', `${SESSION}; ${JOIN}`)
    expect(before.status).toBe(status)
    expect(await text(before)).toContain(title)
    const after = await post('/app/join', {}, {}, `${SESSION}; ${JOIN}`)
    expect(after.status).toBe(status)
    expect(await text(after)).toContain(title)
    // The dead link is forgotten.
    expect(after.headers.getSetCookie().some(c => /^__Host-fc_signin=; .*Max-Age=0/.test(c))).toBe(true)
  })

  it('gives each failure its own words', async () => {
    const seen = new Set<string>()
    for (const [reason, status] of FAILURES) {
      const refusal = () => (reason === 'domain' ? Response.json({ error: 'domain' }, { status }) : Response.json({ error: 'not found', reason }, { status }))
      relay(asVisitor({ 'invites/preview': refusal }))
      const html = await text(await get('/app/join', `${SESSION}; ${JOIN}`))
      seen.add(html.match(/<p class="lede">([^<]*)</)![1])
    }
    expect(seen.size).toBe(FAILURES.length)
  })

  it('does not say a link is dead when the relay is down, and keeps the code for another try', async () => {
    relay(asVisitor({ 'invites/accept': () => new Response('down', { status: 500 }) }))
    const res = await post('/app/join', {}, {}, `${SESSION}; ${JOIN}`)
    expect(res.status).toBe(502)
    expect(res.headers.getSetCookie().some(c => c.startsWith('__Host-fc_signin='))).toBe(false)
  })

  it('treats a join with no code waiting as an invalid link', async () => {
    relay(asVisitor())
    const res = await post('/app/join', {}, {}, SESSION)
    expect(res.status).toBe(404)
    expect(await text(res)).toContain('This invite isn’t valid')
    expect((await get('/app/join', SESSION)).status).toBe(404)
  })

  it('ignores a cookie whose code is not a code', async () => {
    const calls = relay(asVisitor())
    const res = await get('/app/join', `${SESSION}; __Host-fc_signin=${b64({ j: '../../etc' })}`)
    expect(res.status).toBe(404)
    expect(calls.map(c => c.path)).not.toContain('invites/preview')
    expect((await get('/app/join/not-a-code', SESSION)).status).toBe(404)
  })
})

describe('where an invite code can leak', () => {
  it('sends the no-referrer policy on the join and invite pages, and no-store', async () => {
    relay({
      ...asAdmin({ 'teams/invites/create': () => Response.json({ invite: 'i', code: CODE, expiresAt: NOW + DAY }) }),
      'invites/preview': () => Response.json({ team: { id: 't', name: 'T' }, role: 'member', member: false }),
    })
    const responses = [
      await get(`/app/join/${CODE}`),
      await get('/app/join', `${SESSION}; ${JOIN}`),
      await get('/app/join', JOIN),
      await post('/app/teams/team-1/invites', { role: 'member', days: '7', uses: '50' }),
      await get('/app/teams/team-1'),
    ]
    for (const res of responses) {
      expect(res.headers.get('referrer-policy')).toBe('no-referrer')
      expect(res.headers.get('cache-control')).toBe('no-store')
    }
  })

  it('leaves the created link out of the team page afterwards, and the list shows no codes', async () => {
    relay(asAdmin())
    expect(await get('/app/teams/team-1').then(text)).not.toMatch(/[0-9a-f]{64}/)
  })
})
