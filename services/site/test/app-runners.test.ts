import { afterEach, describe, expect, it, vi } from 'vitest'

import worker, { type Env } from '../src/index'
import { runnersPage, auditPage, confirmedSentence, dayOf, holdSentence, shownLabel, stampOf } from '../src/app/runners'
import labels from '../../../test/fixtures/device-labels.json'

// Runners, a runner's keys, My Devices and the audit log on the web (ov-562),
// against a relay the test plays. The relay's shapes are
// services/relay/src/directory/{runners,inventory,teams}.ts's. These pages
// only read: every test that touches the relay records the routes called, and
// one asserts none of them changes anything.

const RELAY = 'https://relay.example'
const ORIGIN = 'https://www.farcooler.com'
const ASSETS = { fetch: async () => new Response('marketing page') } as unknown as Fetcher
const env = { APPCAST_URL: 'x', ASSETS, APP_RELAY_URL: RELAY, APP_WORKOS_CLIENT_ID: 'client_TEST' } as Env

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
const jwt = (claims: Record<string, unknown>) => `${b64({ alg: 'RS256' })}.${b64(claims)}.c2lnbmF0dXJl`
const ACCESS = jwt({ sub: 'user_ME', sid: 'session_01ABC', exp: Math.floor(Date.now() / 1000) + 600 })
const SESSION = `__Host-fc_session=${b64({ a: ACCESS, r: 'refresh_SECRET', c: 'csrf-TEST-0123456789' })}`

const EVIL = '<script>alert(1)</script>'
const EVIL_ATTR = '"><img src=x onerror=alert(2)>'

interface Call {
  path: string
  body: any
}
type Routes = Record<string, (call: Call) => Response | Promise<Response>>

function relay(routes: Routes): Call[] {
  const calls: Call[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input))
      const call = { path: url.pathname.replace('/v1/directory/', ''), body: init?.body ? JSON.parse(String(init.body)) : null }
      calls.push(call)
      const route = routes[call.path]
      return route ? await route(call) : Response.json({ error: 'not found' }, { status: 404 })
    }),
  )
  return calls
}

const FINGERPRINT = 'SHA256:' + 'abcdefgh' + 'x'.repeat(27) + 'IJKLMNOP'
const DEVICE = (over: Record<string, unknown> = {}) => ({
  id: 'dev-1',
  label: 'Work MacBook',
  keyA: 'ssh-ed25519 AAAA',
  keyAFingerprint: FINGERPRINT,
  state: 'trusted',
  createdAt: Date.UTC(2026, 8, 3, 10),
  revokedAt: null,
  ...over,
})
const ME = (role = 'admin', devices: unknown[] = [DEVICE()]) => ({
  user: { id: 'user_ME', email: 'me@example.com' },
  teams: [
    { id: 'personal-1', name: 'Personal', personal: true, role: 'admin' },
    { id: 'team-1', name: 'Platform Infrastructure', personal: false, role },
  ],
  devices,
})
const access = (user: string, role: string, mail: string | null = `${user}@example.com`) => ({ user, email: mail, role, grantedAt: Date.UTC(2026, 8, 5) })
const RUNNER = (over: Record<string, unknown> = {}) => ({
  id: 'run-1',
  name: 'build-host-with-a-really-quite-long-name-in-the-datacenter',
  createdAt: Date.UTC(2026, 8, 1),
  lastSeenAt: Date.UTC(2026, 9, 14, 15, 4),
  version: '1.2.3',
  owner: 'user_ME',
  unclaimed: false,
  access: [access('user_ME', 'owner', 'me@example.com'), access('user_2', 'read')],
  ...over,
})
const KEY = (over: Record<string, unknown> = {}) => ({
  fingerprint: FINGERPRINT,
  comment: 'me@laptop',
  inside: true,
  scope: 'shell',
  file: 'authorized_keys',
  options: '',
  firstSeenAt: Date.UTC(2026, 8, 2),
  label: 'known',
  reason: 'granted',
  user: 'user_ME',
  flagged: false,
  ...over,
})
const KEYS = (keys: unknown[], over: Record<string, unknown> = {}) => ({
  runner: 'run-1',
  reportedAt: Date.UTC(2026, 9, 14, 15, 5),
  lastSeenAt: Date.UTC(2026, 9, 14, 15, 4),
  error: null,
  total: keys.length,
  unparsed: 0,
  unparsedSinceBaseline: 0,
  keys,
  counts: {},
  ...over,
})
const entry = (id: number, action: string, over: Record<string, unknown> = {}) => ({
  id,
  actor: 'user_ME',
  action,
  subject: null,
  detail: null,
  at: Date.UTC(2026, 9, 14, 9, 30),
  ...over,
})
const MEMBERS = {
  team: { id: 'team-1', name: 'Platform Infrastructure', personal: false },
  members: [
    { user: 'user_ME', email: 'me@example.com', role: 'admin', joinedAt: 1 },
    { user: 'user_2', email: 'two@example.com', role: 'member', joinedAt: 1 },
  ],
  removed: [],
}

const routes = (over: Routes = {}, role = 'admin'): Routes => ({
  me: () => Response.json(ME(role)),
  'teams/members': () => Response.json(MEMBERS),
  'teams/runners': () => Response.json({ runners: [RUNNER()] }),
  'runners/keys': () => Response.json(KEYS([KEY()])),
  'teams/audit': () => Response.json({ entries: [] }),
  ...over,
})

const get = (path: string, cookie: string | null = SESSION) => worker.fetch(new Request(`${ORIGIN}${path}`, { headers: cookie ? { cookie } : {} }), env)
const text = async (res: Response) => (await res.text()).replaceAll('<wbr>', '')

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('links from the team and account pages', () => {
  it('gives an admin Runners and Audit Log, and a member only Runners', async () => {
    relay(routes({ 'teams/invites/list': () => Response.json({ invites: [] }) }))
    const admin = await text(await get('/app/teams/team-1'))
    expect(admin).toContain('href="/app/teams/team-1/runners"')
    expect(admin).toContain('href="/app/teams/team-1/audit"')
    relay(routes({}, 'member'))
    const member = await text(await get('/app/teams/team-1'))
    expect(member).toContain('href="/app/teams/team-1/runners"')
    expect(member).not.toContain('/audit')
  })

  it('links My Devices from the teams page', async () => {
    relay(routes())
    expect(await text(await get('/app'))).toContain('href="/app/devices"')
  })
})

describe('the runners page', () => {
  it('lists each runner with its owner, who has access and when it was last heard from', async () => {
    const calls = relay(routes())
    const res = await get('/app/teams/team-1/runners')
    expect(res.status).toBe(200)
    const html = await text(res)
    expect(html).toContain('build-host-with-a-really-quite-long-name-in-the-datacenter')
    expect(html).toContain('Owned by me@example.com')
    expect(html).toContain('2 people have access')
    expect(html).toContain('Last heard from Oct 14, 2026, 3:04 PM UTC')
    expect(html).toContain('href="/app/teams/team-1/runners/run-1"')
    expect(calls.find(c => c.path === 'teams/runners')?.body).toEqual({ team: 'team-1' })
  })

  it('says so for a runner nobody has claimed, and for a team with none', async () => {
    relay(routes({ 'teams/runners': () => Response.json({ runners: [RUNNER({ unclaimed: true, access: [], lastSeenAt: null })] }) }))
    const html = await text(await get('/app/teams/team-1/runners'))
    expect(html).toContain('Nobody has claimed it yet')
    expect(html).toContain('0 people have access')
    expect(html).toContain('Hasn’t been heard from yet')
    relay(routes({ 'teams/runners': () => Response.json({ runners: [] }) }))
    expect(await text(await get('/app/teams/team-1/runners'))).toContain('This team has no runners yet.')
  })

  it('shows a billing member why, and a stranger a not-found page', async () => {
    relay(routes({ 'teams/runners': () => Response.json({ error: 'forbidden' }, { status: 403 }) }, 'billing'))
    const billing = await get('/app/teams/team-1/runners')
    expect(billing.status).toBe(403)
    expect(await text(billing)).toContain('Runners aren’t visible to you')
    relay(routes())
    const stranger = await get('/app/teams/other-team/runners')
    expect(stranger.status).toBe(404)
    expect(await text(stranger)).toContain('Team not found')
  })
})

describe('a runner', () => {
  it('has Access, Keys, Projects and Activity sections, in that order', async () => {
    relay(routes())
    const res = await get('/app/teams/team-1/runners/run-1')
    expect(res.status).toBe(200)
    const html = await text(res)
    const at = ['Access', 'Keys', 'Projects', 'Activity'].map(h => html.indexOf(`<h2>${h}</h2>`))
    expect(at.every(i => i > 0)).toBe(true)
    expect([...at].sort((a, b) => a - b)).toEqual(at)
    expect(html).toContain('me@example.com')
    expect(html).toContain('Owner')
    expect(html).toContain('Read')
    expect(html).toContain('Version 1.2.3')
  })

  it('points a member to the apps to sign in to Claude: its own Claude heading, one line, no button (ov-584)', async () => {
    relay(routes())
    const html = await text(await get('/app/teams/team-1/runners/run-1'))
    const line = '<p class="note">Sign in to Claude from the Far Cooler app on your Mac or phone.</p>'
    expect(html.split(line).length - 1).toBe(1)
    expect(html).toContain(`<h2>Claude</h2>\n${line}\n<h2>Projects</h2>`)
    // Only on the runner's own page: not the list of runners.
    expect(await text(await get('/app/teams/team-1/runners'))).not.toContain('Sign in to Claude')
  })

  it('puts the keys that need a look first, with the key words and why, and folds the known ones', async () => {
    const keys = [
      KEY({ comment: 'fine-key' }),
      KEY({ fingerprint: 'SHA256:' + 'O'.repeat(43), comment: 'ex-employee', label: 'orphaned', reason: 'not_in_team', user: 'user_2' }),
      KEY({ fingerprint: 'SHA256:' + 'P'.repeat(43), comment: 'someone', label: 'planted', reason: 'no_device', user: null, inside: false, firstSeenAt: Date.UTC(2026, 9, 10) }),
    ]
    relay(routes({ 'runners/keys': () => Response.json(KEYS(keys, { unparsedSinceBaseline: 2 })) }))
    const html = await text(await get('/app/teams/team-1/runners/run-1'))
    expect(html).toContain('2 keys need a look.')
    expect(html.indexOf('someone')).toBeLessThan(html.indexOf('ex-employee'))
    expect(html.indexOf('ex-employee')).toBeLessThan(html.indexOf('fine-key'))
    expect(html).toContain('Added outside Far Cooler')
    expect(html).toContain('No one’s device has registered this key.')
    expect(html).toContain('Its owner isn’t in this team anymore.')
    expect(html).toContain('Registered to user_2@example.com')
    expect(html).toContain('SHA256:PPPPPPPP…PPPPPPPP')
    expect(html).toContain('<details class="fold"><summary>1 known key</summary>')
    expect(html).toContain('2 lines in the keys file can’t be read')
    // The caveat from the relay: reported keys are only as honest as the runner.
    expect(html).toContain('doesn’t prove a runner is clean')
    // The relay's codes never reach the page.
    expect(html).not.toMatch(/no_device|not_in_team|planted|orphaned/)
  })

  it('says a runner that never reported has no keys to show, and a failed read is not an empty one', async () => {
    relay(routes({ 'runners/keys': () => Response.json(KEYS([], { reportedAt: null })) }))
    expect(await text(await get('/app/teams/team-1/runners/run-1'))).toContain('This runner hasn’t reported its keys yet.')
    relay(routes({ 'runners/keys': () => Response.json({ error: 'x' }, { status: 500 }) }))
    const html = await text(await get('/app/teams/team-1/runners/run-1'))
    expect(html).toContain('Couldn’t load this runner’s keys.')
    expect(html).not.toContain('Every key matches')
  })

  it('words a key-file error rather than quoting it', async () => {
    relay(routes({ 'runners/keys': () => Response.json(KEYS([KEY()], { error: 'damaged' })) }))
    const html = await text(await get('/app/teams/team-1/runners/run-1'))
    expect(html).toContain('keys file is damaged')
    relay(routes({ 'runners/keys': () => Response.json(KEYS([KEY()], { error: EVIL })) }))
    expect(await text(await get('/app/teams/team-1/runners/run-1'))).not.toContain('<script')
  })

  it('shows an admin the activity about this runner only, and a member none of the log', async () => {
    const entries = [
      entry(3, 'runner.key_appeared', { actor: 'runner:run-1', subject: 'SHA256:zzz', detail: { runner: 'run-1', comment: 'planted-one', inside: false, file: 'authorized_keys', scope: 'shell' } }),
      entry(2, 'member.joined', { subject: 'user_2', detail: { role: 'member' } }),
      entry(1, 'runner.role_removed', { subject: 'user_2', detail: { runner: 'run-1', role: 'read' } }),
    ]
    relay(routes({ 'teams/audit': () => Response.json({ entries }) }))
    const html = await text(await get('/app/teams/team-1/runners/run-1'))
    expect(html).toContain('A key appeared on build-host-with-a-really-quite-long-name-in-the-datacenter: planted-one.')
    expect(html).toContain('me@example.com took two@example.com’s read access')
    expect(html).not.toContain('joined as')
    const calls = relay(routes({}, 'member'))
    const member = await text(await get('/app/teams/team-1/runners/run-1'))
    expect(member).toContain('Team admins can see what changed on this runner.')
    expect(calls.map(c => c.path)).not.toContain('teams/audit')
  })

  it('is not found for a runner that isn’t on the team', async () => {
    relay(routes())
    const res = await get('/app/teams/team-1/runners/run-nope')
    expect(res.status).toBe(404)
    expect(await text(res)).toContain('Runner not found')
  })
})

describe('the audit log', () => {
  it('words each action, names people by email and runners by name, and never shows a code', async () => {
    const entries = [
      entry(9, 'member.role_changed', { subject: 'user_2', detail: { from: 'member', role: 'admin' } }),
      entry(8, 'invite.created', { detail: { role: 'member' } }),
      entry(7, 'runner.registered', { actor: 'user_2', subject: 'run-1', detail: { name: 'build-host' } }),
      entry(6, 'runner.inventory_error', { actor: 'runner:run-1', subject: 'run-1', detail: { error: 'damaged' } }),
      entry(4, 'runner.member_notified', { actor: 'runner:run-1', subject: 'user_2', detail: { runner: 'run-1', kind: 'login' } }),
      entry(5, 'brand.new_action', { actor: 'user_NOBODY', detail: { anything: 1 } }),
    ]
    relay(routes({ 'teams/audit': () => Response.json({ entries }) }))
    const res = await get('/app/teams/team-1/audit')
    expect(res.status).toBe(200)
    const html = await text(res)
    expect(html).toContain('me@example.com changed two@example.com from Member to Admin.')
    expect(html).toContain('me@example.com made an invite link.')
    expect(html).toContain('two@example.com added the runner build-host.')
    expect(html).toContain('build-host-with-a-really-quite-long-name-in-the-datacenter couldn’t read its keys file.')
    expect(html).toContain('build-host-with-a-really-quite-long-name-in-the-datacenter told two@example.com their Claude login stopped.')
    expect(html).not.toContain('member_notified')
    expect(html).toContain('Something changed.')
    expect(html).toContain('Someone · Oct 14, 2026, 9:30 AM UTC')
    expect(html).not.toMatch(/brand\.new_action|role_changed|inventory_error|user_NOBODY/)
  })

  it('is for admins: a member is told so and the log is never asked for', async () => {
    const calls = relay(routes({}, 'member'))
    const res = await get('/app/teams/team-1/audit')
    expect(res.status).toBe(403)
    expect(await text(res)).toContain('Only admins can see the audit log')
    expect(calls.map(c => c.path)).not.toContain('teams/audit')
  })

  it('pages back by the last id, and ignores a `before` that isn’t a number', async () => {
    const page = Array.from({ length: 100 }, (_, i) => entry(500 - i, 'invite.revoked'))
    const calls = relay(routes({ 'teams/audit': () => Response.json({ entries: page }) }))
    const html = await text(await get('/app/teams/team-1/audit'))
    expect(html).toContain('href="/app/teams/team-1/audit?before=401"')
    await get('/app/teams/team-1/audit?before=401')
    expect(calls.filter(c => c.path === 'teams/audit').pop()?.body).toEqual({ team: 'team-1', before: 401 })
    await get('/app/teams/team-1/audit?before=1%20OR%201')
    expect(calls.filter(c => c.path === 'teams/audit').pop()?.body).toEqual({ team: 'team-1' })
    relay(routes({ 'teams/audit': () => Response.json({ entries: page.slice(0, 3) }) }))
    expect(await text(await get('/app/teams/team-1/audit'))).not.toContain('Older')
  })

  it('says so when nothing has been recorded', async () => {
    relay(routes())
    expect(await text(await get('/app/teams/team-1/audit'))).toContain('Nothing has been recorded yet.')
  })
})

describe('My Devices', () => {
  it('says how a device trusted again was confirmed, and what to do with none confirmed (R-75)', async () => {
    const again = DEVICE({ id: 'd4', label: 'Phone', state: 'trusted', createdAt: Date.UTC(2026, 9, 1), trustedAt: Date.UTC(2026, 9, 1), trustReset: true })
    relay(routes({ me: () => Response.json(ME('admin', [again])) }))
    expect(await text(await get('/app/devices'))).toContain('Confirmed when it signed in, since none of your devices was confirmed')
    const stranded = DEVICE({ id: 'd5', label: 'Tablet', state: 'pending' })
    relay(routes({ me: () => Response.json(ME('admin', [stranded, DEVICE({ id: 'd6', state: 'revoked', revokedAt: Date.UTC(2026, 9, 2) })])) }))
    const html = await text(await get('/app/devices'))
    expect(html).toContain('Do that on the device you want to keep')
    expect(html).not.toContain('Scan its code from one of your confirmed devices')
  })

  it('says when a device waiting out a trust reset’s hold can be added by team admins (R-76)', async () => {
    const until = Date.now() + 3 * 86_400_000
    const held = DEVICE({ id: 'd7', label: 'Phone', state: 'trusted', trustReset: true, heldUntil: until })
    const child = DEVICE({ id: 'd8', label: 'Laptop', state: 'certified', heldUntil: until })
    const over = DEVICE({ id: 'd9', label: 'Tablet', state: 'trusted', trustReset: true, heldUntil: Date.now() - 1000 })
    relay(routes({ me: () => Response.json(ME('admin', [held, child, over])) }))
    const html = await text(await get('/app/devices'))
    const sentence = holdSentence(stampOf(until))
    expect(html).toContain(`Confirmed when it signed in, since none of your devices was confirmed. ${sentence}`)
    expect(html).toContain(`Confirmed by another of your devices. ${sentence}`)
    expect(html.split('team admins can').length - 1).toBe(2)
  })

  it('says each team whose admin lifted a device’s hold (ov-565 fix round 1, M1)', async () => {
    const at = Date.UTC(2026, 9, 15)
    const lifted = DEVICE({ id: 'd10', label: 'Phone', state: 'trusted', trustReset: true, holdConfirmations: [{ team: 'Acme\u202e', at }] })
    relay(routes({ me: () => Response.json(ME('admin', [lifted])) }))
    const html = await text(await get('/app/devices'))
    expect(confirmedSentence('Acme', dayOf(at))).toBe('An admin of “Acme” confirmed it on Oct 15, 2026, so that team’s runners can add it now.')
    expect(html).toContain(confirmedSentence('Acme', dayOf(at)))
    expect(html).not.toContain('\u202e')
  })

  it('lists each device with its key words and the day it was added, like the apps', async () => {
    relay(routes({ me: () => Response.json(ME('admin', [DEVICE(), DEVICE({ id: 'd2', label: 'Phone', state: 'pending', keyAFingerprint: 'SHA256:' + 'q'.repeat(43), createdAt: Date.UTC(2026, 9, 1) })])) }))
    const html = await text(await get('/app/devices'))
    expect(html).toContain('Work MacBook')
    expect(html).toContain('SHA256:abcdefgh…IJKLMNOP')
    expect(html).toContain('Added Sep 3, 2026')
    expect(html).toContain('Confirmed as the first device on your account')
    expect(html).toContain('SHA256:qqqqqqqq…qqqqqqqq')
    expect(html).toContain('Not confirmed yet.')
  })

  it('shows a label the way the apps do: no quote marks, 40 characters at most', async () => {
    const label = `"Bob" said “hi” ${'x'.repeat(80)}`
    relay(routes({ me: () => Response.json(ME('admin', [DEVICE({ label }), DEVICE({ id: 'd2', label: '  ' })])) }))
    const html = await text(await get('/app/devices'))
    expect(html).toContain(`Bob said hi ${'x'.repeat(28)}<`)
    expect(html).not.toContain('x'.repeat(29))
    expect(html).toContain('>a device<')
  })

  it('keeps removed devices apart, dated', async () => {
    relay(routes({ me: () => Response.json(ME('admin', [DEVICE(), DEVICE({ id: 'd3', label: 'Old iPad', state: 'revoked', revokedAt: Date.UTC(2026, 9, 2) })])) }))
    const html = await text(await get('/app/devices'))
    expect(html.indexOf('<h2>Removed</h2>')).toBeLessThan(html.indexOf('Old iPad'))
    expect(html).toContain('Removed Oct 2, 2026')
  })

  it('says so with none', async () => {
    relay(routes({ me: () => Response.json(ME('admin', [])) }))
    expect(await text(await get('/app/devices'))).toContain('No device keys yet.')
  })
})

describe('escaping', () => {
  it('escapes every string the relay sends, in a name, a label, a comment, an email or an audit detail', async () => {
    const runner = RUNNER({
      name: EVIL,
      version: EVIL_ATTR,
      access: [access('user_ME', 'owner', `me${EVIL}@example.com`), access('user_2', EVIL, EVIL_ATTR)],
    })
    const keys = KEYS([KEY({ comment: EVIL_ATTR, label: EVIL, reason: EVIL, scope: EVIL, fingerprint: `SHA256:${EVIL}`, user: 'user_ME' }), KEY({ comment: EVIL, label: 'planted', reason: 'no_device' })])
    const entries = [
      entry(4, 'runner.registered', { detail: { name: EVIL } }),
      entry(3, 'team.created', { detail: { name: EVIL_ATTR } }),
      entry(2, 'runner.key_appeared', { detail: { runner: 'run-1', comment: EVIL } }),
      entry(1, EVIL, { actor: EVIL, subject: EVIL_ATTR, detail: { name: EVIL } }),
    ]
    const members = { ...MEMBERS, members: [{ user: 'user_ME', email: `x${EVIL_ATTR}@example.com`, role: 'admin', joinedAt: 1 }] }
    const me = { ...ME('admin', [DEVICE({ label: EVIL, keyAFingerprint: EVIL_ATTR, state: EVIL })]), user: { id: 'user_ME', email: EVIL } }
    me.teams[1].name = EVIL
    relay(
      routes({
        me: () => Response.json(me),
        'teams/members': () => Response.json(members),
        'teams/runners': () => Response.json({ runners: [runner] }),
        'runners/keys': () => Response.json(keys),
        'teams/audit': () => Response.json({ entries }),
      }),
    )
    for (const path of ['/app/teams/team-1/runners', '/app/teams/team-1/runners/run-1', '/app/teams/team-1/audit', '/app/devices', '/app']) {
      const html = await text(await get(path))
      // Nothing the relay sent became a tag or an attribute.
      expect(html, path).not.toContain('<script>alert')
      expect(html, path).not.toContain('<img')
      expect(html, path).not.toContain('onerror=alert(2)>')
      expect(html.replaceAll('&lt;script&gt;', '').includes('<script'), path).toBe(false)
    }
    // And the text itself is there, escaped, where a person reads it.
    const detail = await text(await get('/app/teams/team-1/runners/run-1'))
    expect(detail).toContain('&lt;script&gt;alert(1)&lt;/script&gt;')
    expect(detail).toContain('&quot;&gt;&lt;img src=x onerror=alert(2)&gt;')
    const log = await text(await get('/app/teams/team-1/audit'))
    expect(log).toContain('added the runner &lt;script&gt;alert(1)&lt;/script&gt;.')
  })

  it('escapes the runner id in a link', async () => {
    relay(routes({ 'teams/runners': () => Response.json({ runners: [RUNNER({ id: '"><b>' })] }) }))
    const html = await text(await get('/app/teams/team-1/runners'))
    expect(html).toContain('href="/app/teams/team-1/runners/%22%3E%3Cb%3E"')
  })
})

describe('read-only', () => {
  it('reads from the relay and changes nothing, on every new page', async () => {
    const calls = relay(routes({ 'teams/invites/list': () => Response.json({ invites: [] }) }))
    for (const path of ['/app/teams/team-1/runners', '/app/teams/team-1/runners/run-1', '/app/teams/team-1/audit', '/app/devices']) {
      expect((await get(path)).status, path).toBe(200)
    }
    const READS = new Set(['me', 'teams/runners', 'teams/members', 'runners/keys', 'teams/audit'])
    expect(calls.length).toBeGreaterThan(0)
    for (const call of calls) expect(READS.has(call.path), call.path).toBe(true)
  })

  it('has no form on a page but the sign-out, and takes no POST', async () => {
    relay(routes())
    for (const path of ['/app/teams/team-1/runners', '/app/teams/team-1/runners/run-1', '/app/teams/team-1/audit', '/app/devices']) {
      const html = await text(await get(path))
      expect([...html.matchAll(/<form[^>]*action="([^"]+)"/g)].map(m => m[1]), path).toEqual(['/app/sign-out'])
      const posted = await worker.fetch(new Request(`${ORIGIN}${path}`, { method: 'POST', headers: { origin: ORIGIN, cookie: SESSION }, body: '' }), env)
      expect(posted.status, path).toBe(405)
    }
  })

  it('asks a signed-out visitor to sign in without asking the relay anything', async () => {
    const calls = relay(routes())
    for (const path of ['/app/teams/team-1/runners', '/app/teams/team-1/runners/run-1', '/app/teams/team-1/audit', '/app/devices']) {
      const html = await text(await get(path, null))
      expect(html, path).toContain('href="/app/sign-in"')
    }
    expect(calls).toEqual([])
  })

  it('shows the sign-in page when the relay ends the session, and a retry when it can’t be reached', async () => {
    relay({ me: () => Response.json({ error: 'unauthorized' }, { status: 401 }), })
    // The refresh also fails, so the session is over.
    expect(await text(await get('/app/devices'))).toContain('href="/app/sign-in"')
    relay(routes({ 'teams/runners': () => new Response('x', { status: 503 }) }))
    const res = await get('/app/teams/team-1/runners')
    expect(res.status).toBe(502)
    expect(await text(res)).toContain('Try Again')
  })
})

describe('fix round 1', () => {
  it('words one unreadable line and one key in the singular', async () => {
    const planted = KEY({ label: 'planted', reason: 'no_device', user: null })
    relay(routes({ 'runners/keys': () => Response.json(KEYS([planted], { unparsedSinceBaseline: 1 })) }))
    const html = await text(await get('/app/teams/team-1/runners/run-1'))
    expect(html).toContain('1 line in the keys file can’t be read, and wasn’t there at the first report. sshd may still obey it.')
    expect(html).toContain('1 key needs a look.')
  })

  it('treats a hostile total or count as a number, never as markup', async () => {
    relay(routes({ 'runners/keys': () => Response.json(KEYS([KEY()], { total: EVIL, unparsedSinceBaseline: EVIL_ATTR })) }))
    const html = await text(await get('/app/teams/team-1/runners/run-1'))
    expect(html).not.toContain('<script>alert')
    expect(html).not.toContain('<img')
    expect(html).not.toContain('NaN')
    relay(routes({ 'runners/keys': () => Response.json(KEYS([KEY()], { total: 9e999, unparsedSinceBaseline: -4 })) }))
    expect(await text(await get('/app/teams/team-1/runners/run-1'))).not.toContain('Infinity')
  })

  it('says there are no older entries on an empty older page', async () => {
    relay(routes())
    expect(await text(await get('/app/teams/team-1/audit?before=3'))).toContain('No older entries.')
  })

  it('words an empty Activity by what it looked at, and always links the whole log', async () => {
    relay(routes())
    const html = await text(await get('/app/teams/team-1/runners/run-1'))
    expect(html).toContain('Nothing about this runner is in the team’s latest 100 changes.')
    expect(html).toContain('href="/app/teams/team-1/audit">See the whole audit log')
  })

  it('keeps Access and Keys when the log or the members fail to load', async () => {
    for (const broken of ['teams/audit', 'teams/members']) {
      relay(routes({ [broken]: () => new Response('x', { status: 503 }) }))
      const res = await get('/app/teams/team-1/runners/run-1')
      expect(res.status, broken).toBe(200)
      const html = await text(res)
      expect(html, broken).toContain('Couldn’t load activity.')
      expect(html, broken).toContain('<h2>Access</h2>')
      expect(html, broken).toContain('me@example.com')
      expect(html, broken).toContain('Every key matches a teammate.')
    }
  })

  it('encodes a hostile team id and runner id in every link', () => {
    const team = { id: '"><script>x</script>', name: 'T', personal: false, role: 'admin' }
    const viewer = { email: null, csrf: 'c' }
    const runner = { id: '"><b>', name: 'r', lastSeenAt: null, version: null, unclaimed: false, access: [] }
    for (const html of [runnersPage(team, [runner], viewer), auditPage(team, [], { users: new Map(), runners: new Map() }, 5, true, viewer)]) {
      expect(html).not.toContain('<script>x')
      expect(html).not.toMatch(/href="[^"]*[<>][^"]*"/)
      expect(html).toContain('%22%3E%3Cscript%3Ex%3C%2Fscript%3E')
    }
  })
})

/// One table for every platform (ov-560 R1): the relay's, AgentKit's and
/// Android's tests read it too.
describe('a device label', () => {
  it.each(labels.cases)('is shown without what could masquerade: $why', ({ label, shown }) => {
    expect(shownLabel(label)).toBe(shown)
  })
})
