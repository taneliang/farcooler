import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

// The deployment configuration, as text, so one test below can check what the
// four relays are actually configured to accept. Raw rather than parsed by a
// library: see `varsByEnvironment`.
import wranglerToml from '../wrangler.toml?raw'

// The Android app's own channel ids, as text. The relay now names them, and
// nothing between the two languages would notice if either side moved — see
// `spells both channels the way the Android app creates them`.
import notifierKt from '../../../apps/android/app/src/main/java/com/farcooler/notify/Notifier.kt?raw'

import worker, { ALERT_BODY_BUDGET, ALERT_TITLE_BUDGET, STATE_BUDGET, cut } from '../src/index'
import { anonymousId, record } from '../src/analytics'
import { fingerprintOf, parseEd25519 } from '../src/keys'
import { androidChannel, sendLiveActivity, topicMismatch } from '../src/push'
import { verifySession } from '../src/workos'

/// What the relay must never get wrong.
///
/// This is the security boundary of the whole product and it had no tests at
/// all — CI proved the worker compiled and bundled, which is not the same as
/// proving that a stolen daemon token cannot name its own destination, or that
/// a WorkOS error body does not come back to the caller. Every test here is one
/// of those, not a coverage exercise.
///
/// Real D1, not a fake: the account scoping on revoke and the expiry predicate
/// on notify are SQL, and a hand-written fake would happily agree with whatever
/// this code believed. The schema is the real migration chain — see
/// `test/migrations.ts`.

// MARK: - Signed sessions

/// A real RSA key, so `requireAccount` can really verify a session.
///
/// The signed-in routes were previously tested only for the 401 they give
/// someone with no session, which left everything they actually do — the
/// upserts, the COALESCE that keeps a token an old client does not resend —
/// unexercised. Stubbing `verifySession` would have tested the stub.
const signing = await crypto.subtle.generateKey(
  {
    name: 'RSASSA-PKCS1-v1_5',
    modulusLength: 2048,
    publicExponent: new Uint8Array([1, 0, 1]),
    hash: 'SHA-256',
  },
  true,
  ['sign', 'verify'],
)
const publicJwk = {
  ...(await crypto.subtle.exportKey('jwk', signing.publicKey)),
  kid: 'test-key',
}

/// A key WorkOS has never published, for a token that only LOOKS right.
///
/// It signs under the SAME `kid` as the real one on purpose. A forgery naming a
/// key the JWKS does not carry is refused at the key lookup, before a signature
/// is ever checked — so it would be refused just as firmly by a route that
/// checks no signature at all, and would prove nothing about the one thing
/// these tests are for. This forgery has to reach `crypto.subtle.verify` and be
/// turned away there.
const forgery = await crypto.subtle.generateKey(
  {
    name: 'RSASSA-PKCS1-v1_5',
    modulusLength: 2048,
    publicExponent: new Uint8Array([1, 0, 1]),
    hash: 'SHA-256',
  },
  true,
  ['sign', 'verify'],
)

/// The issuer and client id this test environment's tokens carry, taken from
/// the bindings rather than repeated here: a fixture that hard-coded them would
/// go on passing after someone changed the configuration the worker reads.
const ISSUER = (env as any).WORKOS_ISSUER as string
const CLIENT_ID = (env as any).WORKOS_CLIENT_ID as string

const seconds = () => Math.floor(Date.now() / 1000)

/// A signed token carrying exactly the claims a test names, and nothing else.
///
/// Separate from `sessionFor` because the verifier's whole job is refusing
/// tokens with something missing, and a helper that quietly filled the gaps in
/// could not express a token with a gap in it.
async function signTestJwt(
  claims: Record<string, unknown>,
  key: CryptoKey = signing.privateKey,
): Promise<string> {
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const payload = base64Url(JSON.stringify(claims))
  const signature = await crypto.subtle.sign(
    'RSASSA-PKCS1-v1_5',
    key,
    new TextEncoder().encode(`${header}.${payload}`),
  )
  return `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
}

/// The claims a real AuthKit access token carries, which is what the routes are
/// handed.
///
/// It used to be `sub`, `email` and `exp`. The verifier now requires the issuer,
/// the `client_id` and `iat` as well — so a fixture short of any of them is no
/// longer a session, and every signed-in test below would be testing a 401.
///
/// No `aud` and no `auth_time`, because a real token has neither. A fixture
/// richer than the real thing is how a verifier that refuses production traffic
/// passes its own suite.
async function sessionFor(userId: string): Promise<string> {
  return await signTestJwt(claimsFor(userId))
}

/// The same claims, unsigned, so a test can spoil exactly one of them.
///
/// Split out of `sessionFor` because the two halves of the session boundary
/// need it: the verifier's own tests spoil a claim and expect null, and the
/// route tests spoil a claim and expect a 401. A second copy of this list would
/// drift, and the direction it drifts in is a fixture richer than a real token
/// — which is how a verifier that refuses production traffic passes its suite.
function claimsFor(userId: string, overrides: Record<string, unknown> = {}) {
  const now = seconds()
  return {
    sub: userId,
    email: `${userId}@example.test`,
    iss: ISSUER,
    client_id: CLIENT_ID,
    sid: 'session_test',
    iat: now,
    exp: now + 3600,
    ...overrides,
  }
}

function base64Url(value: string | Uint8Array): string {
  const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value
  return btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replaceAll('=', '')
}

// MARK: - Watching what goes out

interface Call {
  url: string
  headers: Record<string, string>
  body: any
}

/// Every outbound request, with the JWKS answered for free.
///
/// `requireAccount` verifies against WorkOS's published keys, so a test that
/// stubbed fetch without serving them would fail as a 401 rather than as the
/// thing it was written to check.
function watchFetch(reply: (call: Call) => Response | Promise<Response> = ok): Call[] {
  const calls: Call[] = []
  vi.stubGlobal('fetch', async (input: any, init: any = {}) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))

    const call: Call = {
      url,
      headers: init.headers ?? {},
      body: init.body && typeof init.body === 'string' ? JSON.parse(init.body) : null,
    }
    calls.push(call)
    return await reply(call)
  })
  return calls
}

function ok(): Response {
  return new Response('{}')
}

/// The pushes, in order, ignoring the JWKS and anything else on the way.
function pushes(calls: Call[]): Call[] {
  return calls.filter(call => call.url.includes('push.apple.com') || call.url.includes('fcm'))
}

function post(path: string, body: unknown, bearer?: string): Promise<Response> {
  return postAs({}, path, body, bearer)
}

/// The same request against a worker whose BINDINGS differ from the suite's.
///
/// A misconfigured deployment is not a misconfigured request: the relay reads
/// `CHANNEL` and `APNS_TOPIC` off `env`, so the only way to express one is to
/// hand a route a different env. Overridden per call rather than declared in
/// `vitest.config.ts`, because the bindings there are the CORRECT pairing and
/// every other test in this file has to keep running against a relay that is
/// configured properly.
function postAs(
  bindings: Record<string, unknown>,
  path: string,
  body: unknown,
  bearer?: string,
  headers: Record<string, string> = {},
): Promise<Response> {
  return worker.fetch(
    new Request(`https://relay.test${path}`, {
      method: 'POST',
      headers: { ...(bearer ? { authorization: `Bearer ${bearer}` } : {}), ...headers },
      body: JSON.stringify(body),
    }),
    { ...env, ...bindings } as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
}

async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))
  return [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2, '0')).join('')
}

/// Give an account a machine holding `token`.
async function pair(account: string, token: string, expiresAt: number | null = null) {
  await env.DB.prepare(
    `INSERT INTO accounts (id, created_at) VALUES (?, ?) ON CONFLICT (id) DO NOTHING`,
  )
    .bind(account, Date.now())
    .run()
  await env.DB.prepare(
    `INSERT INTO daemons (id, account_id, token_hash, label, created_at, expires_at)
     VALUES (?, ?, ?, ?, ?, ?)`,
  )
    .bind(crypto.randomUUID(), account, await sha256(token), 'Studio', Date.now(), expiresAt)
    .run()
}

/// Register a device the way the app does, through the route.
async function register(account: string, fields: Record<string, unknown> = {}) {
  const response = await post(
    '/v1/devices',
    { platform: 'apns', pushToken: 'device-token', ...fields },
    await sessionFor(account),
  )
  return response
}

/// A roster row belonging to somebody else.
///
/// Every account clause on `live_activities` was a `WHERE` nothing in this file
/// could notice. With one account's rows in the table, a query that dropped its
/// scoping returned exactly the same rows, deleted exactly the same rows, and
/// every assertion below went on passing — so the three clauses that keep one
/// person's fleet off another person's lock screen were guarded by nothing.
/// This is the second account. The tests that call it assert its rows are
/// neither read, drawn, counted, nor deleted.
async function foreignAgent(
  account: string,
  terminal: string,
  fields: { status?: string; updatedAt?: number } = {},
) {
  const at = fields.updatedAt ?? Date.now()
  await env.DB.prepare(
    `INSERT INTO accounts (id, created_at) VALUES (?, ?) ON CONFLICT (id) DO NOTHING`,
  )
    .bind(account, Date.now())
    .run()
  // Written through SQL rather than through `/v1/notify`, because a notice
  // needs a paired machine and a session, and what is wanted here is only the
  // row: the point is what this account's request does to somebody else's
  // table, not how that row got there.
  await env.DB.prepare(
    `INSERT INTO live_activities
       (id, account_id, terminal, update_token, environment, updated_at,
        label, machine, status, detail, insertions, deletions, commits,
        started_at, status_since)
     VALUES (?, ?, ?, '', NULL, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)`,
  )
    .bind(
      crypto.randomUUID(),
      account,
      terminal,
      at,
      'theirs',
      'Their Mac',
      fields.status ?? 'blocked',
      'Not your question',
      7,
      3,
      2,
      at,
    )
    .run()
}

/// A card belonging to somebody else's install.
///
/// The counterpart of `foreignAgent`, needed for the same reason and for a
/// sharper one. `install_cards` is `UNIQUE (account_id)`, so a table holding
/// this account's card holds exactly ONE ROW — and against one row an
/// account-scoped write and an unscoped one do precisely the same thing. Every
/// write in this service that touches this table was therefore a `WHERE` that
/// no test in this file could possibly notice: the two on
/// `/v1/devices/activity`, the one on `/v1/notify/retire`, and the three on
/// `/v1/notify`. All six could be dropped together and the suite stayed green.
///
/// What they read as in production is worth spelling out, because it is not a
/// leak of somebody's data but a denial of everybody's card: one person swiping
/// theirs away sets EVERY account's card to the sentinel, and one runner going
/// quiet takes down every lock screen in the fleet.
async function foreignCard(
  account: string,
  fields: { updateToken?: string; leaderTerminal?: string; leaderStatus?: string } = {},
) {
  const at = Date.now()
  const card = {
    update_token: fields.updateToken ?? 'their-update-token',
    leader_terminal: fields.leaderTerminal ?? 'their-term',
    leader_status: fields.leaderStatus ?? 'working',
    dismissed_at: null,
    updated_at: at,
    pushed_at: at,
  }
  await env.DB.prepare(
    `INSERT INTO accounts (id, created_at) VALUES (?, ?) ON CONFLICT (id) DO NOTHING`,
  )
    .bind(account, at)
    .run()
  await env.DB.prepare(
    `INSERT INTO install_cards
       (id, account_id, update_token, environment, leader_terminal, leader_status,
        dismissed_at, updated_at, pushed_at)
     VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?)`,
  )
    .bind(
      crypto.randomUUID(),
      account,
      card.update_token,
      card.leader_terminal,
      card.leader_status,
      card.dismissed_at,
      card.updated_at,
      card.pushed_at,
    )
    .run()
  return card
}

/// One account's card, in every column any write in this service touches.
///
/// The whole row rather than the field a given test happened to think of: the
/// six writes set different columns, and a per-column assertion would have to
/// be right about which one each of them reaches.
async function cardOf(account: string) {
  return await env.DB.prepare(
    `SELECT update_token, leader_terminal, leader_status, dismissed_at, updated_at, pushed_at
     FROM install_cards WHERE account_id = ?`,
  )
    .bind(account)
    .first<{
      update_token: string
      leader_terminal: string | null
      leader_status: string | null
      dismissed_at: number | null
      updated_at: number
      pushed_at: number | null
    }>()
}

/// The terminals still on one account's roster, in a fixed order.
async function roster(account: string): Promise<string[]> {
  const rows = await env.DB.prepare(
    `SELECT terminal FROM live_activities WHERE account_id = ? ORDER BY terminal`,
  )
    .bind(account)
    .all<{ terminal: string }>()
  return (rows.results ?? []).map(row => row.terminal)
}

/// What one value costs on the wire, which is the only unit either cap is in.
function bytes(value: unknown): number {
  return new TextEncoder().encode(JSON.stringify(value)).length
}

beforeEach(async () => {
  for (const table of ['install_cards', 'live_activities', 'devices', 'daemons', 'accounts']) {
    await env.DB.prepare(`DELETE FROM ${table}`).run()
  }
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('the shape of the API', () => {
  it('answers nothing but POST', async () => {
    const response = await worker.fetch(
      new Request('https://relay.test/v1/notify'),
      env as never,
      {} as never,
    )
    expect(response.status).toBe(405)
  })

  it('does not invent routes', async () => {
    expect((await post('/v1/nope', {})).status).toBe(404)
  })
})

// MARK: - The issuer the relays are configured to accept

/// The `iss` a real AuthKit access token from these WorkOS projects carries.
///
/// A LITERAL, and that is the whole point of it. It was recorded on 2026-08-19
/// by decoding a live canary access token — base64url the middle segment of the
/// JWT and read `iss` out of the JSON — so it comes from a token WorkOS minted
/// and from nothing this repository believes.
///
/// Everything else in this file mints its tokens with `iss: ISSUER`, where
/// `ISSUER` is the same binding `verifySession` compares against. That is
/// circular: it agrees with the right value, with the bare host, and with
/// `https://example.invalid` alike. It is not a flaw in those tests — they are
/// about the verifier, and the verifier should be handed the configured issuer
/// — but it does mean nothing here could ever notice a wrong CONFIGURATION.
/// Something did go wrong there: `WORKOS_ISSUER` was `https://api.workos.com`
/// in all four blocks of wrangler.toml from eb56857 (Aug 17) until 2026-08-19,
/// which 401'd every session-authenticated route on every channel, and this
/// suite was green for both of those days.
///
/// If WorkOS ever changes the form, this test is supposed to fail: somebody
/// decodes a fresh token and updates the line below, deliberately, rather than
/// the suite quietly following the configuration wherever it goes.
const RECORDED_CANARY_ISSUER =
  'https://api.workos.com/user_management/client_01M03MZKRH5ZPR160DATT149WX'

/// Which is to say: the host, `/user_management/`, and the client id.
const ISSUER_PREFIX = 'https://api.workos.com/user_management/'

/// The `[vars]` of each environment in wrangler.toml, keyed by channel.
///
/// Twelve lines rather than a TOML dependency, because what is wanted is two
/// string values out of four flat blocks, and a parser in devDependencies to
/// read them is a supply chain in exchange for a regular expression. The
/// environment names are asserted below, so a parse that matched nothing fails
/// loudly instead of passing every remaining expectation vacuously.
function varsByEnvironment(): Record<string, Record<string, string>> {
  const blocks: Record<string, Record<string, string>> = {}
  let current: Record<string, string> | null = null
  for (const line of wranglerToml.split('\n')) {
    const header = line.match(/^\[(?:env\.([A-Za-z0-9_]+)\.)?vars\]\s*$/)
    if (header) {
      current = blocks[header[1] ?? 'stable'] = {}
    } else if (line.startsWith('[')) {
      current = null
    } else {
      const pair = line.match(/^([A-Z_]+)\s*=\s*"([^"]*)"\s*$/)
      if (current && pair) current[pair[1]] = pair[2]
    }
  }
  return blocks
}

describe('the issuer this relay accepts', () => {
  const environments = varsByEnvironment()

  it('is configured for all four channels', () => {
    // Guards every expectation below: if the parse stopped matching, each
    // `for` over an empty object would pass without checking anything.
    expect(Object.keys(environments).sort()).toEqual(['canary', 'local', 'preview', 'stable'])
    for (const [channel, vars] of Object.entries(environments)) {
      expect(vars.WORKOS_CLIENT_ID, channel).toMatch(/^client_[A-Z0-9]+$/)
      expect(vars.WORKOS_ISSUER, channel).toBeTypeOf('string')
    }
  })

  it('matches, on canary, the issuer a real canary token carries', () => {
    // The one comparison against evidence. Everything else in this describe is
    // consistency; this is the anchor.
    expect(environments.canary.WORKOS_ISSUER).toBe(RECORDED_CANARY_ISSUER)
    expect(RECORDED_CANARY_ISSUER).toBe(
      `${ISSUER_PREFIX}${environments.canary.WORKOS_CLIENT_ID}`,
    )
  })

  it("is each channel's own client id, not a bare host and not another channel's", () => {
    // Per environment because the path segment IS the client id, so there is no
    // one right answer to share — and `vars` are not inherited by wrangler
    // environments, so a channel that omitted this would deploy with none.
    for (const [channel, vars] of Object.entries(environments)) {
      expect(vars.WORKOS_ISSUER, channel).toBe(`${ISSUER_PREFIX}${vars.WORKOS_CLIENT_ID}`)
    }
  })

  it('is the shape every fixture in this file is minted with', () => {
    // So the tokens the rest of the suite signs are the shape of a real one. A
    // fixture in a shape production never sends is how a verifier that refuses
    // production traffic passes its own suite.
    expect(ISSUER).toBe(`${ISSUER_PREFIX}${CLIENT_ID}`)
  })
})

// MARK: - The claims the authorization rests on

/// What a valid signature is not.
///
/// A signature says WorkOS minted this token. It does not say the token was
/// minted for THIS application, or that it has not expired — and every route
/// below treats `sub` as an account it is about to hand data to. These are the
/// checks that turn a verified signature into a session.
///
/// Every fixture here is the shape of a REAL AuthKit token, because the failure
/// this suite has to catch is not only the forged token that gets in: it is
/// equally the honest one that does not. A verifier demanding a claim WorkOS
/// never sends refuses every sign-in on the channel, and a suite whose fixtures
/// invented that claim would report it green.
describe('session verification', () => {
  /// The claims a real access token carries, so each test can spoil exactly one.
  function wellFormed(overrides: Record<string, unknown> = {}) {
    return claimsFor('user_1', overrides)
  }

  it('refuses a token with no expiry', async () => {
    // The old verifier checked `exp` only when it was there, so a token without
    // one was a session that never ended.
    watchFetch()
    const { exp, ...claims } = wellFormed()
    expect(await verifySession(await signTestJwt(claims), env as never)).toBeNull()
  })

  it('refuses a token that has expired', async () => {
    watchFetch()
    const token = await signTestJwt(wellFormed({ exp: seconds() - 3600 }))
    expect(await verifySession(token, env as never)).toBeNull()
  })

  it('refuses a token from another issuer', async () => {
    watchFetch()
    const token = await signTestJwt(wellFormed({ iss: 'https://evil.example' }))
    expect(await verifySession(token, env as never)).toBeNull()
  })

  it('refuses a token minted for another application', async () => {
    // The sharpest one. Two applications in the same WorkOS environment share a
    // key set, so the other application's token verifies here — and used to
    // authenticate as its `sub`, against an account it had nothing to do with.
    // `client_id` is where AuthKit records which application asked; there is no
    // `aud` to compare.
    watchFetch()
    const token = await signTestJwt(wellFormed({ client_id: 'client_someone_else' }))
    expect(await verifySession(token, env as never)).toBeNull()
  })

  it('refuses a token that names no application at all', async () => {
    watchFetch()
    const { client_id, ...claims } = wellFormed()
    expect(await verifySession(await signTestJwt(claims), env as never)).toBeNull()
  })

  it('refuses a token that is not yet valid', async () => {
    watchFetch()
    const token = await signTestJwt(wellFormed({ nbf: seconds() + 3600 }))
    expect(await verifySession(token, env as never)).toBeNull()
  })

  it('refuses a token with no subject', async () => {
    watchFetch()
    const token = await signTestJwt(wellFormed({ sub: '' }))
    expect(await verifySession(token, env as never)).toBeNull()
  })

  it('accepts a token that never says when anyone authenticated', async () => {
    // The real-world shape, and the reason this test exists rather than its
    // opposite. A WorkOS access token carries no `auth_time` at all, so a
    // verifier that demanded one would refuse every genuine session — and `iat`
    // cannot stand in for it, because this relay mints fresh access tokens from
    // refresh tokens at `/v1/auth/refresh`. The onboarding confirmation's
    // freshness comes from LocalAuthentication on the device instead, which is
    // about the person holding the phone rather than about a claim.
    watchFetch()
    const claims = wellFormed()
    expect('auth_time' in claims).toBe(false)
    expect((await verifySession(await signTestJwt(claims), env as never))?.userId).toBe('user_1')
  })

  it('accepts a well-formed token and says who it belongs to', async () => {
    watchFetch()
    const token = await signTestJwt(wellFormed())
    expect(await verifySession(token, env as never)).toEqual({
      userId: 'user_1',
      email: 'user_1@example.test',
    })
  })

  it('accepts an audience list that contains this application', async () => {
    // A token with no `client_id` but an `aud` — the shape a custom AuthKit
    // domain or a later token format might arrive in. `aud` is a string OR an
    // array of them, per RFC 7519, so comparing the raw claim would refuse the
    // array form outright.
    watchFetch()
    const { client_id, ...claims } = wellFormed()
    const token = await signTestJwt({ ...claims, aud: ['client_other', CLIENT_ID] })
    expect((await verifySession(token, env as never))?.userId).toBe('user_1')
  })

  it('refuses an audience list that does not', async () => {
    watchFetch()
    const { client_id, ...claims } = wellFormed()
    const token = await signTestJwt({ ...claims, aud: ['client_other'] })
    expect(await verifySession(token, env as never)).toBeNull()
  })
})

// MARK: - That a route actually RUNS the verifier

/// The other half of the session boundary, and the half nothing held.
///
/// `verifySession` is exercised as a unit above, exhaustively. What was never
/// exercised is that a ROUTE runs it. Every signed-in test in this file mints
/// its token with `sessionFor`, which is always valid, and the only 401 anyone
/// asserted was for a request carrying no `authorization` header at all — and a
/// header is not a signature. So `requireAccount` could have base64-decoded the
/// payload and trusted `sub`, throwing away the signature, the expiry, the
/// issuer and the `client_id` together, and every test in this file would have
/// stayed green: the unit tests would have gone on testing `verifySession`, and
/// nothing at all would have been testing that anything calls it.
///
/// These are the tokens that tell the two apart. Each decodes to a perfectly
/// good `sub`, and each has to be refused for a reason only a checked signature
/// or a checked claim can supply.
describe('the session a signed-in route insists on', () => {
  /// What the attempt was answered with, and what it left behind.
  ///
  /// The status alone is the weaker half. `requireAccount` creates the account
  /// row for whatever `sub` it believed BEFORE the route body runs, so a
  /// weakened verification leaves a stranger's account in the database even on
  /// a request that goes on to fail for some other reason — and that row is
  /// what every other table in this service hangs off.
  async function attempt(token?: string) {
    const response = await post(
      '/v1/devices',
      { platform: 'apns', pushToken: 'device-token' },
      token,
    )
    const accounts = await env.DB.prepare(`SELECT id FROM accounts`).all<{ id: string }>()
    const devices = await env.DB.prepare(`SELECT id FROM devices`).all<{ id: string }>()
    return {
      status: response.status,
      accounts: (accounts.results ?? []).map(row => row.id),
      devices: (devices.results ?? []).length,
    }
  }

  const refused = { status: 401, accounts: [] as string[], devices: 0 }

  it('takes a token signed by the key WorkOS publishes', async () => {
    // The control, and it is not decoration: without it every assertion below
    // would also be satisfied by a route that refused everything, which is the
    // other way to make this file green and the one that takes the product
    // down for every real user at once.
    watchFetch()
    expect(await attempt(await sessionFor('user_1'))).toEqual({
      status: 200,
      accounts: ['user_1'],
      devices: 1,
    })
  })

  it('refuses a token signed by a key WorkOS never published', async () => {
    // The forgery: right shape, right claims, right `kid`, and the one thing
    // that cannot be manufactured is wrong. A route that decodes its payload
    // rather than verifying it cannot tell this from the control above.
    watchFetch()
    const token = await signTestJwt(claimsFor('user_1'), forgery.privateKey)
    expect(await attempt(token)).toEqual(refused)
  })

  it('refuses a token with nothing in its signature at all', async () => {
    // `header.payload.` — three segments with an empty third. This is what
    // somebody writes by hand after seeing one real token, and it is exactly
    // what a payload-decoding route accepts.
    watchFetch()
    const [header, payload] = (await sessionFor('user_1')).split('.')
    expect(await attempt(`${header}.${payload}.`)).toEqual(refused)
  })

  it('refuses a token that expired an hour ago', async () => {
    // Genuinely signed by WorkOS, and finished. A stolen token is worth
    // whatever is left of its lifetime and nothing after it, which is a
    // property of this check and of no other.
    watchFetch()
    const token = await signTestJwt(claimsFor('user_1', { exp: seconds() - 3600 }))
    expect(await attempt(token)).toEqual(refused)
  })

  it('refuses a genuine token from another issuer', async () => {
    watchFetch()
    const token = await signTestJwt(claimsFor('user_1', { iss: 'https://evil.example' }))
    expect(await attempt(token)).toEqual(refused)
  })

  it('refuses a genuine token minted for another application', async () => {
    // The one a key set cannot catch on its own: two applications in one WorkOS
    // environment are signed with the same keys, so the neighbour's token
    // verifies here, and the only thing standing between it and this account's
    // data is the `client_id` comparison.
    watchFetch()
    const token = await signTestJwt(claimsFor('user_1', { client_id: 'client_someone_else' }))
    expect(await attempt(token)).toEqual(refused)
  })

  it('refuses a caller carrying no authorization at all', async () => {
    // The case this file already had, kept here beside the five it could not
    // tell itself apart from.
    watchFetch()
    expect(await attempt()).toEqual(refused)
  })
})

describe('/v1/auth/token', () => {
  it('refuses an exchange with no PKCE verifier', async () => {
    // Without the verifier there is nothing binding the code to the app that
    // started sign-in, and `farcooler://` is a scheme any app can claim.
    expect((await post('/v1/auth/token', { code: 'abc' })).status).toBe(400)
  })

  it('turns a WorkOS rejection into a 401 and leaks none of its body', async () => {
    // The body of a failed grant can echo the submitted credential back. The
    // top-level handler already refuses to return error text for this reason;
    // this path used to log the whole thing.
    vi.stubGlobal(
      'fetch',
      async () => new Response(JSON.stringify({ error: 'invalid_grant', code: 'SECRET' }), {
        status: 400,
      }),
    )
    const response = await post('/v1/auth/token', { code: 'abc', verifier: 'v' })
    expect(response.status).toBe(401)
    expect(await response.text()).not.toContain('SECRET')
  })

  it('creates the account on the way through', async () => {
    vi.stubGlobal(
      'fetch',
      async () => new Response(
        JSON.stringify({
          access_token: 'at',
          refresh_token: 'rt',
          user: { id: 'user_1', email: 'someone@example.test' },
        }),
      ),
    )
    const body = await (await post('/v1/auth/token', { code: 'abc', verifier: 'v' })).json<{
      accessToken: string
      userId: string
    }>()
    expect(body.accessToken).toBe('at')
    expect(body.userId).toBe('user_1')

    // Signing in is what creates the account, not registering a device. A
    // person who signs in and never grants push permission still exists.
    const account = await env.DB.prepare(`SELECT email FROM accounts WHERE id = ?`)
      .bind('user_1')
      .first<{ email: string }>()
    expect(account?.email).toBe('someone@example.test')
  })
})

describe('/v1/auth/refresh and /v1/auth/logout', () => {
  it('refuses a refresh with no token', async () => {
    expect((await post('/v1/auth/refresh', {})).status).toBe(400)
  })

  it('refuses a logout with no token', async () => {
    expect((await post('/v1/auth/logout', {})).status).toBe(400)
  })

  it('reports a logout as done even when WorkOS is unreachable', async () => {
    // The device is clearing its copy either way. An error here would only
    // teach the app to keep a credential when the server is having a bad day.
    vi.stubGlobal('fetch', async () => {
      throw new Error('network')
    })
    const response = await post('/v1/auth/logout', { refreshToken: 'rt' })
    expect(response.status).toBe(200)
  })
})

// MARK: - The one protection that only exists in production

/// `withinRate` fails open with no binding, and this suite declares none.
///
/// The absence is deliberate and `vitest.config.ts` says so: these tests are
/// about the routes, and a real limiter in front of every one of them would
/// throttle the suite rather than the thing being tested. **That decision is
/// not the same as its consequence.** What followed from it was that nothing
/// anywhere exercised the throttle at all — the whole `startsWith('/v1/auth/')`
/// arm could be deleted, or made to gate the wrong routes, or made to refuse
/// everybody, and 169 tests stayed green while the only unauthenticated surface
/// this service has went unprotected in production and over-protected nowhere.
///
/// The decision stands. The limiter is injected PER REQUEST, so the suite's own
/// bindings still declare none and every other test in this file runs against a
/// relay with no throttle, exactly as before.
describe('the throttle on /v1/auth/*', () => {
  /// A `RateLimit` binding that answers the same way every time, and remembers
  /// what it was asked about.
  function limiter(success: boolean) {
    const asked: { key: string }[] = []
    return {
      asked,
      async limit(options: { key: string }) {
        asked.push(options)
        return { success }
      },
    }
  }

  const AUTH = ['/v1/auth/token', '/v1/auth/refresh', '/v1/auth/logout']

  it('refuses with a 429 and does no work at all', async () => {
    // Before any work, which is the whole point: two of these three spend the
    // relay's WorkOS API key per request, so a caller that could make the relay
    // spend it and only then be refused would still be exhausting the upstream
    // quota that everybody's sign-in depends on.
    const calls = watchFetch()
    const gate = limiter(false)

    const response = await postAs(
      { AUTH_LIMIT: gate },
      '/v1/auth/token',
      { code: 'c', codeVerifier: 'v' },
    )

    expect(response.status).toBe(429)
    expect(await response.json()).toEqual({ error: 'slow down' })
    expect(calls).toEqual([])
    expect(gate.asked.length).toBe(1)
  })

  it('gates all three of them, and only them', async () => {
    // The prefix, said as the set it actually covers. Everything below these
    // needs a session or a machine token and is throttled by having to have
    // one; putting them behind the IP limiter as well would let one office
    // network's notifications throttle each other.
    const gate = limiter(false)
    for (const path of AUTH) {
      watchFetch()
      expect((await postAs({ AUTH_LIMIT: gate }, path, {})).status, path).toBe(429)
    }
    expect(gate.asked.length).toBe(3)

    const open = limiter(false)
    for (const path of ['/v1/devices', '/v1/daemons', '/v1/notify', '/v1/notify/retire']) {
      watchFetch()
      const response = await postAs({ AUTH_LIMIT: open }, path, {})
      expect(response.status, path).not.toBe(429)
    }
    // Not merely a different status: the limiter was never consulted.
    expect(open.asked).toEqual([])
  })

  it('keys on the connecting IP, which is all an unauthenticated caller has', async () => {
    // Not a strong identity — a botnet has many — but it is what stops one
    // client burning the WorkOS quota, which is the realistic failure. A
    // constant key would throttle every caller together, and the first person to
    // hold the button down would lock everyone else out of signing in.
    const gate = limiter(true)
    watchFetch()
    await postAs({ AUTH_LIMIT: gate }, '/v1/auth/logout', {}, undefined, {
      'cf-connecting-ip': '203.0.113.7',
    })
    await postAs({ AUTH_LIMIT: gate }, '/v1/auth/logout', {}, undefined, {
      'cf-connecting-ip': '198.51.100.9',
    })
    // A request with no such header is Cloudflare not having set one, which is
    // one bucket for all of them rather than a free pass.
    await postAs({ AUTH_LIMIT: gate }, '/v1/auth/logout', {})

    expect(gate.asked.map(each => each.key)).toEqual(['203.0.113.7', '198.51.100.9', 'unknown'])
  })

  it('lets a caller under the limit straight through', async () => {
    // Without this the test above passes against a relay that answers 429 to
    // everything.
    watchFetch()
    const gate = limiter(true)
    const response = await postAs({ AUTH_LIMIT: gate }, '/v1/auth/logout', { refreshToken: 'r' })

    expect(response.status).toBe(200)
    expect(gate.asked.length).toBe(1)
  })

  it('fails OPEN when there is no limiter at all', async () => {
    // A local `wrangler dev` has no rate-limit binding, and a relay that refused
    // every sign-in because one was missing would be a worse outage than the one
    // being prevented. This is the suite's own configuration, so it is also what
    // every other test in this file relies on.
    watchFetch()
    expect((env as any).AUTH_LIMIT).toBeUndefined()
    const response = await post('/v1/auth/logout', { refreshToken: 'r' })
    expect(response.status).toBe(200)
  })

  it('is declared on every channel that is deployed', async () => {
    // The other half of "a change that disabled it would be invisible". The
    // code path is held above; this is the binding it needs to exist at all,
    // and deleting it from one channel's block would leave that relay silently
    // unthrottled.
    // `[[unsafe.bindings]]` for stable and `[[env.<channel>.unsafe.bindings]]`
    // for the other three, which is how wrangler spells a per-environment
    // binding — and the shape the four blocks of this file already use.
    const blocks = wranglerToml
      .split(/^\[\[(?:env\.[a-z]+\.)?unsafe\.bindings\]\]$/m)
      .slice(1)
    const limiters = blocks.filter(block => /name = "AUTH_LIMIT"/.test(block))
    expect(limiters.length).toBe(4)
    for (const block of limiters) {
      expect(block).toContain('type = "ratelimit"')
      expect(block).toMatch(/simple = \{ limit = \d+, period = \d+ \}/)
    }
  })
})

describe('/v1/notify', () => {
  it('refuses a request with no token', async () => {
    expect((await post('/v1/notify', { title: 'hi' })).status).toBe(401)
  })

  it('refuses a token nobody issued', async () => {
    await pair('user_1', 'good')
    expect((await post('/v1/notify', { title: 'hi' }, 'guessed')).status).toBe(401)
  })

  it('refuses a token that has expired', async () => {
    await pair('user_1', 'stale', Date.now() - 1000)
    expect((await post('/v1/notify', { title: 'hi' }, 'stale')).status).toBe(401)
  })

  it('still accepts a pairing made before expiries existed', async () => {
    // NULL means no expiry. Backfilling those would log people out of a feature
    // they had just set up, to introduce a policy.
    await pair('user_1', 'old', null)
    expect((await post('/v1/notify', { title: 'hi' }, 'old')).status).toBe(200)
  })

  it('will not deliver to an account other than its own', async () => {
    // The rule the whole design rests on: the daemon says "notify my user" and
    // never names a destination, so a stolen token is worth one thing —
    // notifying the phone of the person it was stolen from.
    await pair('user_1', 'mine')
    await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES (?, ?)`)
      .bind('user_2', Date.now())
      .run()
    await env.DB.prepare(
      `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    )
      .bind(crypto.randomUUID(), 'user_2', 'apns', 'someone-elses-phone', 'Their phone', Date.now())
      .run()

    const body = await (await post('/v1/notify', { title: 'hi' }, 'mine')).json<{
      delivered: number
    }>()
    expect(body.delivered).toBe(0)
  })

  it('records what the machine is running', async () => {
    await pair('user_1', 'mine')
    await post('/v1/notify', { title: 'hi', version: '0.2.0+abc123' }, 'mine')
    const row = await env.DB.prepare(`SELECT version, last_seen_at FROM daemons`).first<{
      version: string
      last_seen_at: number
    }>()
    expect(row?.version).toBe('0.2.0+abc123')
    expect(row?.last_seen_at).toBeGreaterThan(0)
  })

  it('needs something to say', async () => {
    await pair('user_1', 'mine')
    expect((await post('/v1/notify', {}, 'mine')).status).toBe(400)
  })
})

// MARK: - "When an agent finishes or fails", off

/// The toggle that only ever worked while the app was open.
///
/// It has always been read by each app's own in-process notifier, which by
/// definition runs when the app is running — and the case this product exists
/// for is the phone in a pocket, where the banner is drawn from a push this
/// route sends. So the setting meant silence with the app open and a banner
/// otherwise: the common case was the broken one.
///
/// Two things must survive the fix, and both are here because both have been
/// broken before. A `blocked` agent has stopped and is waiting for an answer;
/// reading this column on that branch would take the product's one promise away
/// from someone who only silenced the endings. And the `done` push is the only
/// thing that has ever taken a Live Activity card down, so skipping it wholesale
/// leaves a lock screen reading "Working" over an agent that stopped ten minutes
/// ago — the bug `5398dba` was written to close.
describe('a device that has turned finishing off', () => {
  /// Give the account a card that is up, addressable, and leading `terminal`.
  ///
  /// Written straight into the row rather than raised through a `working`
  /// notify, so the pushes a test counts are only the ones the `done` produced.
  async function carded(terminal: string) {
    await env.DB.prepare(
      `INSERT INTO install_cards
         (id, account_id, update_token, leader_terminal, leader_status, updated_at)
       VALUES (?, 'user_1', 'update-token', ?, 'working', ?)`,
    )
      .bind(crypto.randomUUID(), terminal, Date.now())
      .run()
  }

  /// The alerts, which are the pushes this preference is about. A Live Activity
  /// push goes to the same host and is told apart by its type.
  function alerts(calls: Call[]): Call[] {
    return pushes(calls).filter(call => call.headers['apns-push-type'] !== 'liveactivity')
  }

  it('is skipped on a finished agent while the other device is not', async () => {
    // The case the whole change is about. One account, two phones, one of them
    // opted out: the push goes to exactly one of them, and to the right one.
    const calls = watchFetch()
    await register('user_1', { pushToken: 'quiet-phone', notifyOnDone: false })
    await register('user_1', { pushToken: 'loud-phone', notifyOnDone: true })
    await pair('user_1', 'mine')

    const response = await post(
      '/v1/notify',
      { title: 'claude finished', terminal: 'term-1', status: 'done' },
      'mine',
    )

    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ delivered: 1 })
    const sent = alerts(calls)
    expect(sent.length).toBe(1)
    expect(sent[0].url).toContain('/device/loud-phone')
  })

  it('still has its Live Activity card taken down', async () => {
    // Skip the ALERT, never the notify. `done` is the only thing that retires a
    // card, and a device that silenced the banner and kept the card would be
    // left with a lock screen reading "Working" over an agent that has stopped.
    const calls = watchFetch()
    await register('user_1', {
      pushToken: 'quiet-phone',
      notifyOnDone: false,
      liveActivityStartToken: 'start-token',
    })
    await pair('user_1', 'mine')
    await carded('term-1')

    const response = await post(
      '/v1/notify',
      { title: 'claude finished', terminal: 'term-1', status: 'done' },
      'mine',
    )

    // Nothing buzzed, and 200 with nothing delivered is the established
    // contract — the `working` branch has always answered this way, and the
    // daemon checks the status, not the count.
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ delivered: 0 })
    expect(alerts(calls).length).toBe(0)

    const ended = pushes(calls).filter(call => call.headers['apns-push-type'] === 'liveactivity')
    expect(ended.length).toBe(1)
    expect(ended[0].body.aps.event).toBe('end')
    // And the row goes with the run, or this install is refused a card forever.
    expect(await env.DB.prepare(`SELECT id FROM install_cards`).first()).toBe(null)
  })

  it('is still told an agent needs it', async () => {
    // The failure mode this column must not have. Someone who silenced the
    // endings has said nothing about an agent that stopped and is waiting, which
    // is the notification the product exists to deliver.
    const calls = watchFetch()
    await register('user_1', { pushToken: 'quiet-phone', notifyOnDone: false })
    await pair('user_1', 'mine')

    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    expect(alerts(calls).length).toBe(1)
  })

  it('is a device that said so, not one that has never been asked', async () => {
    // An absent field reads as notify. Every row that existed before migration
    // 0007 has NULL here, as does every registration from a build too old to
    // send it — and the alternative is that deploying this silences every
    // installed app until it happens to update.
    const calls = watchFetch()
    await register('user_1', { pushToken: 'old-build' })
    await pair('user_1', 'mine')

    await post('/v1/notify', { title: 'claude finished', status: 'done' }, 'mine')

    expect(alerts(calls).length).toBe(1)
  })

  it('keeps its answer when an older build re-registers over it', async () => {
    // The COALESCE, for the same reason `version` and the push-to-start token
    // have one: a build that predates the field re-registering the same phone
    // must not erase what a newer one reported. Getting this wrong un-silences
    // somebody who deliberately went quiet.
    watchFetch()
    await register('user_1', { notifyOnDone: false })
    await register('user_1', { label: 'Renamed' })

    const row = await env.DB.prepare(
      `SELECT label, notify_on_done FROM devices`,
    ).first<{ label: string; notify_on_done: number | null }>()
    expect(row?.label).toBe('Renamed')
    expect(row?.notify_on_done).toBe(0)
  })

  it('changes its mind by re-registering', async () => {
    // What the apps do when the toggle flips. Registration runs when a push
    // token arrives, not when a preference changes, so each app re-registers on
    // the setter — without that the relay's copy goes stale and the setting
    // appears to do nothing.
    const calls = watchFetch()
    await register('user_1', { notifyOnDone: false })
    await register('user_1', { notifyOnDone: true })
    await pair('user_1', 'mine')

    await post('/v1/notify', { title: 'claude finished', status: 'done' }, 'mine')

    expect(pushes(calls).length).toBe(1)
  })
})

// MARK: - What the phone's extension is handed

/// The three fields that decide whether the widgets are updated at all.
///
/// `mutable-content`, and the top-level `status` and `label`. Get any of them
/// wrong and the notification service extension never runs, or runs and gives
/// up on its first guard — and NOTHING reports it. The banner still arrives
/// looking exactly right, the relay still counts the delivery, and the entire
/// NSE-to-widget half of this feature is off. There is no log to find it in
/// because nothing failed.
describe('the alert push body', () => {
  it('asks iOS to run the notification service extension', async () => {
    // Without `mutable-content` the extension is not invoked at all, so the
    // widgets stay on whatever the app last wrote — which on a phone nobody has
    // opened today is nothing whatsoever.
    const calls = watchFetch()
    await register('user_1')
    await pair('user_1', 'mine')
    await post(
      '/v1/notify',
      { title: 'claude needs you', subtitle: 'Create haiku.txt?', terminal: 'term-1', status: 'blocked', label: 'claude' },
      'mine',
    )

    const [alert] = pushes(calls)
    expect(alert.headers['apns-push-type']).toBe('alert')
    expect(alert.body.aps['mutable-content']).toBe(1)
  })

  it('names the agent and its status beside the banner, not inside it', async () => {
    // The extension reads these two and nothing else — it has no terminal and
    // no fleet — so it folds the push into the snapshot by them. Renamed or
    // dropped, `didReceive` falls through its first guard and returns having
    // changed nothing, silently. They are top level rather than inside `aps`
    // because `aps` is Apple's dictionary and custom keys in it are ignored.
    const calls = watchFetch()
    await register('user_1')
    await pair('user_1', 'mine')
    await post(
      '/v1/notify',
      { title: 'claude needs you', subtitle: 'Create haiku.txt?', terminal: 'term-1', status: 'blocked', label: 'claude' },
      'mine',
    )

    const [alert] = pushes(calls)
    expect(alert.body.terminal).toBe('term-1')
    expect(alert.body.status).toBe('blocked')
    expect(alert.body.label).toBe('claude')
    // And the banner itself is untouched by any of it: the machine composed
    // that sentence and the extension is told not to rewrite it.
    expect(alert.body.aps.alert).toEqual({
      title: 'claude needs you',
      body: 'Create haiku.txt?',
    })
    expect(alert.body.aps['thread-id']).toBe('term-1')
    // Time-sensitive, so a blocked agent can break a Focus: it has stopped and
    // stays stopped until answered, which is what that level is for.
    expect(alert.body.aps['interruption-level']).toBe('time-sensitive')
    expect(alert.headers['apns-priority']).toBe('10')
  })

  it('carries how a turn ended, which the status cannot say', async () => {
    // `done` is "the agent stopped", not "the agent succeeded" — a turn that
    // died arrives with exactly the same status as one that finished. The
    // extension picks its mark off the status word and nothing else, and
    // `accessoryCircular` draws only that mark, so a dropped `failed` is a `✓`
    // on a lock screen widget for an agent that died.
    const calls = watchFetch()
    await register('user_1')
    await pair('user_1', 'mine')
    await post(
      '/v1/notify',
      { title: 'cursor failed', subtitle: "Its last turn didn’t finish", terminal: 'term-1', status: 'done', label: 'cursor', failed: true },
      'mine',
    )

    const [alert] = pushes(calls)
    expect(alert.body.failed).toBe(true)
    // And a daemon that sends nothing gets undefined, which the extension reads
    // as false: the behavior it always had.
    const older = watchFetch()
    await post('/v1/notify', { title: 'claude finished', terminal: 'term-2', status: 'done' }, 'mine')
    expect(pushes(older)[0].body.failed).toBeUndefined()
  })

  it('leaves status and label off for a daemon that sends neither', async () => {
    // The compatibility promise, at the field level. An older daemon's push must
    // still be a valid alert; the extension simply finds no status, gives up on
    // its first guard, and the banner is delivered exactly as it always was.
    const calls = watchFetch()
    await register('user_1')
    await pair('user_1', 'mine')
    await post('/v1/notify', { title: 'Agent stopped', terminal: 'term-1' }, 'mine')

    const [alert] = pushes(calls)
    expect(alert.body.aps['mutable-content']).toBe(1)
    expect(alert.body.status).toBeUndefined()
    expect(alert.body.label).toBeUndefined()
  })
})

// MARK: - Android

/// The Android push body, which had no test at all.
///
/// The gap this closes is not a coverage number. `sendFcm` built `data: {
/// terminal }` and nothing else, `FarCoolerMessagingService` read
/// `data["activity"]`, and neither end had ever seen the other's bytes — so a
/// key no producer sent could sit on the reading end for the whole life of the
/// feature with every test on both sides passing. `FCM_SERVICE_ACCOUNT` was the
/// empty string in `vitest.config.ts` until now, which meant `sendFcm` died in
/// `JSON.parse` before it composed anything: this suite could not have caught
/// it even had it tried.
///
/// The stake is the one high-importance channel. `agents.blocked` is the only
/// notification this app sends that is allowed past a Focus, and it exists for
/// the case the whole product exists for — an agent stopped three time zones
/// away, waiting, on a phone in a pocket.
describe('the Android push body', () => {
  /// A phone registered the way the Android app registers one.
  const android = { platform: 'fcm', pushToken: 'android-token' }

  /// The message Google is handed, unwrapped.
  function fcm(calls: Call[]): any {
    return pushes(calls).find(call => call.url.includes('fcm.googleapis.com'))?.body.message
  }

  it('puts a blocked agent on the channel that may break a Focus', async () => {
    // The defect, end to end. This is a `notification` message, so on a phone
    // whose app is backgrounded or dead — the only case this push exists for —
    // Firebase draws the tray card itself and `onMessageReceived` never runs.
    // `android.notification.channel_id` is the single field that decides where
    // that card lands, and without it every push took the manifest default,
    // which names the quiet channel.
    const calls = watchFetch()
    await register('user_1', android)
    await pair('user_1', 'mine')
    await post(
      '/v1/notify',
      { title: 'claude needs you', subtitle: 'Create haiku.txt?', terminal: 'term-1', status: 'blocked', label: 'claude' },
      'mine',
    )

    const message = fcm(calls)
    expect(message.android.notification.channel_id).toBe('agents.blocked')
    // And the same word in `data`, for the other half of the split: an app in
    // the foreground gets `onMessageReceived` and picks its own channel from
    // this, through `NotificationCopy.channelFor`. Two paths, one word.
    expect(message.data.status).toBe('blocked')
    expect(message.android.priority).toBe('HIGH')
  })

  it('leaves a finished agent on the quiet channel', async () => {
    // The half that must NOT change. Over-alerting every finished agent breaks
    // a Focus for the normal case, and that is the failure people answer by
    // turning the whole app off — which costs them the blocked ones too.
    const calls = watchFetch()
    await register('user_1', android)
    await pair('user_1', 'mine')
    await post(
      '/v1/notify',
      { title: 'claude finished', subtitle: 'Both tests pass.', terminal: 'term-1', status: 'done', failed: false },
      'mine',
    )

    const message = fcm(calls)
    expect(message.android.notification.channel_id).toBe('agents.done')
    expect(message.data.status).toBe('done')
  })

  it('sends the pane and the state, and nothing else about the work', async () => {
    // What may cross. This lands on a lock screen and passes through Google's
    // servers, so the whole of `data` is pinned rather than spot-checked: a
    // terminal is a UUID the runner minted and a status is one of three fixed
    // words, and both are strictly less than the agent's own scraped sentence
    // that `notification` already carries to the same screen.
    //
    // `label` and `failed` are on the payload, were sent to Apple in the same
    // request, and are deliberately absent here — nothing on Android reads
    // either, and a field nobody reads is a field on a lock screen for nothing.
    const calls = watchFetch()
    await register('user_1', android)
    await pair('user_1', 'mine')
    await post(
      '/v1/notify',
      { title: 'cursor failed', subtitle: 'add-auth — Its last turn didn’t finish', terminal: 'term-1', status: 'done', label: 'cursor', failed: true },
      'mine',
    )

    const message = fcm(calls)
    expect(message.data).toEqual({ terminal: 'term-1', status: 'done' })
    // The sentence the daemon composed, unrewritten, because Firebase draws it.
    expect(message.notification).toEqual({
      title: 'cursor failed',
      body: 'add-auth — Its last turn didn’t finish',
    })
  })

  it('sends no status at all for a daemon that sends none', async () => {
    // An absent status stays absent rather than going as `""`. The daemon says
    // it outright — an empty or invented status is worse than none — and a
    // runner too old to send one must get exactly the behavior it always got,
    // which is the quiet channel and a push that still arrives.
    const calls = watchFetch()
    await register('user_1', android)
    await pair('user_1', 'mine')
    await post('/v1/notify', { title: 'Agent stopped', terminal: 'term-1' }, 'mine')

    const message = fcm(calls)
    expect(message.data).toEqual({ terminal: 'term-1' })
    expect(message.android.notification.channel_id).toBe('agents.done')
  })

  it('leaves the Apple push untouched', async () => {
    // Both phones are served by one `/v1/notify` and one `Payload`, so the risk
    // in giving Android a field is giving it to iOS and the watch as well —
    // every Apple device registers as `apns`, and the notification service
    // extension refuses a body it cannot decode by silently changing nothing.
    // The Android keys live inside `message`, which APNs never sees.
    const calls = watchFetch()
    await register('user_1')
    await register('user_1', android)
    await pair('user_1', 'mine')
    await post(
      '/v1/notify',
      { title: 'claude needs you', subtitle: 'Create haiku.txt?', terminal: 'term-1', status: 'blocked', label: 'claude' },
      'mine',
    )

    const apple = pushes(calls).find(call => call.url.includes('push.apple.com'))!.body
    expect(apple.aps['mutable-content']).toBe(1)
    expect(apple.aps['interruption-level']).toBe('time-sensitive')
    expect(apple.status).toBe('blocked')
    expect(apple.label).toBe('claude')
    expect(apple.android).toBeUndefined()
    expect(apple.message).toBeUndefined()
    // And the Android message carries none of Apple's dictionary either.
    expect(fcm(calls).aps).toBeUndefined()
  })

  it('treats a status it has never heard of as news that can wait', async () => {
    // The daemon ships separately and will eventually send a status this relay
    // does not know. Falling back to the loud channel would let a word nobody
    // has written yet break a Focus; falling back to the quiet one costs a
    // notification that arrives without a sound.
    expect(androidChannel('blocked')).toBe('agents.blocked')
    expect(androidChannel('done')).toBe('agents.done')
    expect(androidChannel('working')).toBe('agents.done')
    expect(androidChannel('nudged')).toBe('agents.done')
    expect(androidChannel(undefined)).toBe('agents.done')
  })

  it('spells both channels the way the Android app creates them', async () => {
    // The other end of a contract with no compiler across it. These two ids are
    // `Notifier.CHANNEL_BLOCKED` and `CHANNEL_DONE`, created at launch by
    // `createChannels`, and FCM DROPS a notification naming a channel the app
    // has not created — so a typo here is not a wrong channel, it is a push
    // that arrives nowhere at all and reports success.
    expect(notifierKt).toContain('const val CHANNEL_BLOCKED = "agents.blocked"')
    expect(notifierKt).toContain('const val CHANNEL_DONE = "agents.done"')
  })
})

// MARK: - What a refused push does

/// Every analytics event a request produced, in order.
///
/// Spied on the real binding rather than injected. The wiring being checked is
/// exactly the wiring an injected fake would replace: a `fetch` the push service
/// answered with a 400, through `sendPush`'s boolean, to the counter that names
/// it. `record` puts the event name and the platform in `blobs` and the outcome
/// in `doubles` — see `analytics.ts`.
function watchMetrics(): { name: string; platform: string; ok: number }[] {
  const events: { name: string; platform: string; ok: number }[] = []
  vi.spyOn((env as any).METRICS, 'writeDataPoint').mockImplementation((point: any) => {
    events.push({ name: point.blobs[0], platform: point.blobs[1], ok: point.doubles[0] })
  })
  return events
}

/// Refuse the pushes and answer everything else — the JWKS, and Google's token
/// endpoint — normally.
///
/// Selective on purpose. `googleAccessToken` caches its answer for half an hour
/// in module scope, so a reply function that 400'd every outbound request would
/// poison that cache for whichever test happened to run next.
function refusing(host: string): (call: Call) => Response {
  return call =>
    call.url.includes(host) ? new Response('{"reason":"BadDeviceToken"}', { status: 400 }) : ok()
}

/// A push service refusing is the failure this product cannot afford to be
/// quiet about, and it was the failure nothing observed: all three transports
/// end `return response.ok`, and two of them could be changed to `return true`
/// with the whole suite still green. What that boolean feeds is the count the
/// daemon is answered with and the counter that separates a delivery from a
/// refusal — so a relay whose pushes were all being rejected reported a healthy
/// delivery rate and told every machine its notification had landed.
describe('a push the service refuses', () => {
  const android = { platform: 'fcm', pushToken: 'android-token' }

  it('is not counted as delivered, and is counted as failed', async () => {
    const calls = watchFetch(refusing('push.apple.com'))
    await register('user_1')
    await pair('user_1', 'mine')

    // Spied here rather than at the top, so what follows is every event the
    // notification produced and not merely the ones a filter kept.
    const events = watchMetrics()
    const response = await post('/v1/notify', { title: 'hi' }, 'mine')

    // It really was attempted — this is not a test of a push that never went.
    expect(pushes(calls).length).toBe(1)
    expect(await response.json()).toEqual({ delivered: 0 })
    expect(events).toEqual([{ name: 'notification_failed', platform: 'apns', ok: 0 }])
  })

  it('is not counted as delivered on Android either', async () => {
    // The same boolean on the other transport. `sendFcm` reaches a different
    // service, over a different credential, and returns into the same counter.
    const calls = watchFetch(refusing('fcm.googleapis.com'))
    await register('user_1', android)
    await pair('user_1', 'mine')

    const events = watchMetrics()
    const response = await post('/v1/notify', { title: 'hi' }, 'mine')

    expect(pushes(calls).length).toBe(1)
    expect(await response.json()).toEqual({ delivered: 0 })
    expect(events).toEqual([{ name: 'notification_failed', platform: 'fcm', ok: 0 }])
  })

  it('does not take the other phone down with it', async () => {
    // Per device, inside the loop. One refusal must not stop the loop or spoil
    // the count for a device that was served — a person with a phone and a watch
    // loses both to one dead token otherwise.
    watchFetch(refusing('push.apple.com'))
    await register('user_1')
    await register('user_1', android)
    await pair('user_1', 'mine')

    const events = watchMetrics()
    const response = await post('/v1/notify', { title: 'hi' }, 'mine')

    expect(await response.json()).toEqual({ delivered: 1 })
    expect(events.map(event => `${event.platform}:${event.name}`)).toEqual([
      'apns:notification_failed',
      'fcm:notification_sent',
    ])
  })

  it('counts a refused Live Activity apart from a refused alert', async () => {
    // The reason `activity_failed` exists at all: these fail for reasons the
    // alert cannot — an update token that outlived its activity, a payload the
    // app's ContentState will not decode — and folding them into
    // `notification_failed` would make the delivery rate that actually matters
    // look worse than it is. Here the alert is DELIVERED and only the card's
    // push is refused, which is the case that would be indistinguishable.
    const calls = watchFetch(call =>
      call.body?.aps?.event ? new Response('{"reason":"BadDeviceToken"}', { status: 400 }) : ok(),
    )
    await register('user_1')
    await pair('user_1', 'mine')
    await post(
      '/v1/devices/activity',
      { terminal: 'term-1', updateToken: 'update-token' },
      await sessionFor('user_1'),
    )

    const events = watchMetrics()
    const response = await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    // Two pushes went: the alert, which was taken, and the card's update, which
    // was not.
    expect(pushes(calls).length).toBe(2)
    expect(await response.json()).toEqual({ delivered: 1 })
    expect(events.map(event => event.name)).toEqual(['notification_sent', 'activity_failed'])
    // And the refusal did not turn a delivered notification into a 500. The
    // daemon would retry it and interrupt the person twice for one event.
    expect(response.status).toBe(200)
  })
})

describe('the signed-in routes', () => {
  /// Every route that requires a session.
  ///
  /// `/v1/daemons/revoke` was missing from this list, which is most of how it
  /// came to have no test of any kind — the list is the only place in this file
  /// that names the signed-in routes together, so a route absent from it is a
  /// route nobody notices is absent.
  const paths = [
    '/v1/devices',
    '/v1/devices/activity',
    '/v1/devices/lookup',
    '/v1/devices/verify',
    '/v1/daemons',
    '/v1/account',
    '/v1/devices/revoke',
    '/v1/daemons/revoke',
  ]

  it('refuse a caller with no session', async () => {
    for (const path of paths) {
      expect((await post(path, {})).status, path).toBe(401)
    }
  })

  it('refuse a caller holding a token WorkOS did not sign', async () => {
    // Not the same test as the one above it. A missing header is refused by the
    // first line of `requireAccount`; a forged one is refused only by the
    // verification underneath — and it was the verification nothing covered.
    // Swept over every route rather than proven on one, because the guard is a
    // single function but the ways to lose it are per-route: an early return, a
    // route that reads `sub` for itself, a handler added without the call.
    watchFetch()
    const token = await signTestJwt(claimsFor('user_1'), forgery.privateKey)
    for (const path of paths) {
      expect((await post(path, {}, token)).status, path).toBe(401)
    }
  })
})

// MARK: - Which APNs

describe('the APNs environment', () => {
  it('sends a development build its notification at the sandbox host', async () => {
    // The live bug. A locally-signed build has `aps-environment: development`,
    // so APNs issues it a SANDBOX token, and production APNs answers a sandbox
    // token with BadDeviceToken — push was silently dead for every dev build.
    const calls = watchFetch()
    await register('user_1', { environment: 'development' })
    await pair('user_1', 'mine')
    await post('/v1/notify', { title: 'hi' }, 'mine')

    expect(pushes(calls)[0].url).toBe('https://api.sandbox.push.apple.com/3/device/device-token')
  })

  it('treats a device that never said as production', async () => {
    // Every device registered before the column existed is one of these, which
    // is why NULL means production rather than being backfilled.
    const calls = watchFetch()
    await register('user_1')
    await pair('user_1', 'mine')
    await post('/v1/notify', { title: 'hi' }, 'mine')

    expect(pushes(calls)[0].url).toBe('https://api.push.apple.com/3/device/device-token')
    const row = await env.DB.prepare(`SELECT environment FROM devices`).first<{
      environment: string | null
    }>()
    expect(row?.environment).toBe(null)
  })

  it('keeps what a newer build reported when an older one re-registers', async () => {
    // The COALESCE, and the one place where losing it kills push outright
    // rather than changing a preference. NULL reads as production — see
    // `apnsHost` — so a sandbox device whose column was reset by an older
    // build's registration gets every push posted to the production service,
    // which answers a sandbox token with BadDeviceToken. Nothing reports that:
    // the relay counts a failed delivery and the phone simply stays quiet.
    //
    // The sibling of `keeps a push-to-start token a later registration does not
    // repeat` and `keeps its answer when an older build re-registers over it`,
    // asserted through the HOST as well as the column, because the column is
    // only worth keeping for what it decides.
    const calls = watchFetch()
    await register('user_1', { environment: 'development' })
    await register('user_1', { label: 'Renamed' })
    await pair('user_1', 'mine')
    await post('/v1/notify', { title: 'hi' }, 'mine')

    const row = await env.DB.prepare(`SELECT label, environment FROM devices`).first<{
      label: string
      environment: string | null
    }>()
    expect(row?.label).toBe('Renamed')
    expect(row?.environment).toBe('development')
    expect(pushes(calls)[0].url).toBe('https://api.sandbox.push.apple.com/3/device/device-token')
  })

  it('takes the new answer when a build that knows the field names one', async () => {
    // The other direction, which the COALESCE must not swallow: a device really
    // can move between the two services — a TestFlight build replacing a local
    // one on the same phone — and a registration that NAMES an environment is
    // the newer report, not the older one.
    const calls = watchFetch()
    await register('user_1', { environment: 'development' })
    await register('user_1', { environment: 'production' })
    await pair('user_1', 'mine')
    await post('/v1/notify', { title: 'hi' }, 'mine')

    expect(pushes(calls)[0].url).toBe('https://api.push.apple.com/3/device/device-token')
  })

  it('refuses an environment that is neither', async () => {
    watchFetch()
    expect((await register('user_1', { environment: 'staging' })).status).toBe(400)
  })
})

// MARK: - The two kinds of Live Activity token

describe('/v1/devices', () => {
  it('keeps a push-to-start token a later registration does not repeat', async () => {
    // Same reason `version` is preserved: a build that predates the field
    // re-registering the same phone must not erase what a newer one reported.
    watchFetch()
    await register('user_1', { liveActivityStartToken: 'start-token' })
    await register('user_1', { label: 'Renamed' })

    const row = await env.DB.prepare(
      `SELECT label, live_activity_start_token FROM devices`,
    ).first<{ label: string; live_activity_start_token: string | null }>()
    expect(row?.label).toBe('Renamed')
    expect(row?.live_activity_start_token).toBe('start-token')
  })
})

describe('/v1/devices/activity', () => {
  it('remembers the token for the activity now running', async () => {
    watchFetch()
    const response = await post(
      '/v1/devices/activity',
      { terminal: 'term-1', updateToken: 'update-token', environment: 'development' },
      await sessionFor('user_1'),
    )
    expect(response.status).toBe(200)

    const row = await env.DB.prepare(`SELECT * FROM install_cards`).first<any>()
    expect(row?.update_token).toBe('update-token')
    expect(row?.environment).toBe('development')
    // Nothing is remembered about which agent this card is about, because the
    // app is not the side that knows: the leader is whatever the relay last
    // pushed, and a token filed for a card the relay holds no history of leaves
    // it NULL for the next push to adopt.
    expect(row?.leader_terminal).toBe(null)
    expect(row?.leader_status).toBe(null)
  })

  it('replaces the token when the install starts another activity', async () => {
    // APNs issues a fresh update token per activity and the old one is dead, so
    // a second row would be a card nobody can reach. There is one card per
    // install now, so the terminals below are two agents on one card rather than
    // two cards — and either way this is the same one row.
    watchFetch()
    const session = await sessionFor('user_1')
    await post('/v1/devices/activity', { terminal: 'term-1', updateToken: 'first' }, session)
    await post('/v1/devices/activity', { terminal: 'term-2', updateToken: 'second' }, session)

    const rows = await env.DB.prepare(`SELECT update_token FROM install_cards`).all<any>()
    expect(rows.results?.length).toBe(1)
    expect(rows.results?.[0].update_token).toBe('second')
  })

  it('forgets the activity when the app says it is over', async () => {
    watchFetch()
    const session = await sessionFor('user_1')
    await post('/v1/devices/activity', { terminal: 'term-1', updateToken: 'update' }, session)
    await post('/v1/devices/activity', { terminal: 'term-1', updateToken: null }, session)

    const rows = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(rows.results?.length).toBe(0)
  })

  it('no longer needs to be told which terminal', async () => {
    // The row was keyed on it and is keyed on the install now, so the field this
    // route once 400'd for is one it has nothing to do with. Accepted and
    // ignored rather than refused, because an app talking to a relay that
    // predates the rekey still has to send it and this relay still has to take
    // it from an app that does — ignoring a field is the only compatible way to
    // retire one.
    watchFetch()
    const response = await post(
      '/v1/devices/activity',
      { updateToken: 'u' },
      await sessionFor('user_1'),
    )
    expect(response.status).toBe(200)
    const row = await env.DB.prepare(`SELECT update_token FROM install_cards`).first<any>()
    expect(row?.update_token).toBe('u')
  })

  it('refuses an environment it does not know', async () => {
    watchFetch()
    expect(
      (
        await post(
          '/v1/devices/activity',
          { terminal: 't', updateToken: 'u', environment: 'staging' },
          await sessionFor('user_1'),
        )
      ).status,
    ).toBe(400)
  })
})

// MARK: - Live Activity pushes

describe('/v1/notify and Live Activities', () => {
  /// The account from `register`, a machine called Studio, and a phone that has
  /// offered a push-to-start token.
  async function ready(fields: Record<string, unknown> = {}) {
    await register('user_1', { liveActivityStartToken: 'start-token', ...fields })
    await pair('user_1', 'mine')
  }

  async function running(terminal: string, token = 'update-token', environment?: string) {
    await post(
      '/v1/devices/activity',
      { terminal, updateToken: token, environment },
      await sessionFor('user_1'),
    )
  }

  /// One push's content state, with the live clocks stamped out.
  ///
  /// Every row carries `updatedAt`, which is `Date.now()` at the moment the
  /// relay stored it, so an exact-shape assertion could not be exact without
  /// this. Exactness is worth keeping: the whole point of these `toEqual`s is
  /// that the payload is a fixed contract with a Swift type in another
  /// repository, and `toMatchObject` would pass a card that had quietly lost a
  /// field.
  function stateOf(activity: any) {
    const state = { ...activity.body.aps['content-state'] }
    if (state.rows) {
      state.rows = state.rows.map((row: any) => ({ ...row, updatedAt: '<now>' }))
    }
    return state
  }

  /// The row the relay stores for one agent, as it appears on the card.
  ///
  /// A helper because a row repeats on every card assertion below and the
  /// interesting part of each test is one field of it. `updatedAt` is the stamp
  /// `stateOf` replaces.
  function row(fields: Record<string, unknown>) {
    return {
      terminal: 'term-1',
      label: 'claude',
      machine: 'Studio',
      status: 'blocked',
      detail: 'Waiting for your answer',
      updatedAt: '<now>',
      ...fields,
    }
  }

  it('sends only the alert for a daemon that has never heard of statuses', async () => {
    // The compatibility promise. A daemon built before this change sends
    // neither `status` nor `label` and must behave exactly as it did.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'Agent stopped', terminal: 'term-1' }, 'mine')

    const sent = pushes(calls)
    expect(sent.length).toBe(1)
    expect(sent[0].headers['apns-push-type']).toBe('alert')
    expect(sent[0].url).toContain('/device/device-token')
  })

  it('ignores a status it does not understand', async () => {
    // A newer daemon inventing a status must not cost the user the alert, which
    // is the part that is actually guaranteed.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'hi', terminal: 'term-1', status: 'pondering' }, 'mine')

    expect(pushes(calls).length).toBe(1)
  })

  it('starts an activity on the push-to-start token when none is running', async () => {
    // The whole point of the push-to-start token: nothing on the phone is awake
    // to start the activity itself when the agent goes blocked.
    const calls = watchFetch()
    await ready({ environment: 'development' })
    await post(
      '/v1/notify',
      {
        title: 'Agent needs you',
        subtitle: 'Waiting for your answer',
        terminal: 'term-1',
        status: 'blocked',
        label: 'refactor-auth',
      },
      'mine',
    )

    const [, activity] = pushes(calls)
    expect(activity.url).toBe('https://api.sandbox.push.apple.com/3/device/start-token')
    expect(activity.headers['apns-topic']).toBe('com.farcooler.ios.push-type.liveactivity')
    expect(activity.headers['apns-push-type']).toBe('liveactivity')
    expect(activity.headers['apns-priority']).toBe('10')

    const aps = activity.body.aps
    expect(aps.event).toBe('start')
    // Seconds, not milliseconds. APNs discards an activity push whose timestamp
    // is not older than the last one it saw, and a millisecond value is a year
    // in the fifty-seven thousands.
    expect(aps.timestamp).toBeLessThan(2_000_000_000)
    expect(aps.timestamp).toBeGreaterThan(1_600_000_000)
    // The whole card: the headline, the fleet it heads, and a row per agent.
    // Everything that used to be the activity's IDENTITY is here instead,
    // because the card is per install now and the agent it headlines changes
    // over its life — attributes are fixed for an activity's whole life and
    // APNs rejects a push that repeats them.
    expect(stateOf(activity)).toEqual({
      terminal: 'term-1',
      label: 'refactor-auth',
      machine: 'Studio',
      status: 'blocked',
      detail: 'Waiting for your answer',
      // The header's three numbers, over every agent this account has and not
      // over the rows that fit. The card writes its own sentence out of them.
      blocked: 1,
      review: 0,
      working: 0,
      // Nobody the card has no line for.
      more: 0,
      rows: [row({ label: 'refactor-auth' })],
    })
    // A fixed contract with the app: this string names the Swift type.
    expect(aps['attributes-type']).toBe('AgentActivityAttributes')
    // And all that is left of the attributes: which SHAPE the card was started
    // in, so an app upgraded while an older card is still in flight can tell
    // them apart and end the one it understands least. A rank, not a flag: 1 is
    // terminal-scoped, 2 headlines one agent, 3 carries a row each.
    expect(aps.attributes).toEqual({ version: 3 })
    // Stale after an hour, never dismissed. Nothing reports an update token for
    // a card the relay started while the app was closed, so `done` can arrive
    // to find no row and the card would otherwise claim "Needs You" forever. A
    // dismissal date instead would delete the notification the product exists
    // for, in the case where the agent really is still blocked.
    expect(aps['stale-date']).toBe(aps.timestamp + 3600)
    expect(aps['dismissal-date']).toBeUndefined()
  })

  it('puts the leader\'s turn clock in the state of the card it starts', async () => {
    // The card counts elapsed time from this and nothing else: iOS renders a
    // date as a native timer, so there is no push per tick and no state to
    // update. Drop it and the timer is simply absent — no error, no failed
    // delivery, nothing in a log to find.
    //
    // In the STATE rather than the attributes, which is where it rode when a
    // card was about one terminal. A card whose leader changes has to change the
    // clock with it, or the second agent's work counts from the first agent's
    // start.
    const calls = watchFetch()
    await ready()
    await post(
      '/v1/notify',
      {
        title: 'claude needs you',
        terminal: 'term-1',
        status: 'blocked',
        label: 'claude',
        startedAt: 1_755_000_000_000,
      },
      'mine',
    )

    const [, activity] = pushes(calls)
    expect(activity.body.aps.event).toBe('start')
    expect(stateOf(activity)).toEqual({
      terminal: 'term-1',
      label: 'claude',
      machine: 'Studio',
      status: 'blocked',
      detail: 'Waiting for your answer',
      startedAt: 1_755_000_000_000,
      blocked: 1,
      review: 0,
      working: 0,
      more: 0,
      rows: [row({ startedAt: 1_755_000_000_000 })],
    })
    // A NUMBER, in milliseconds. The app's decoder tells seconds from
    // milliseconds apart by magnitude and reads a string back as nil, which
    // costs the timer without costing the card — the one failure mode that
    // reports itself nowhere.
    expect(typeof activity.body.aps['content-state'].startedAt).toBe('number')
  })

  it('sends no clock at all for a machine that named none', async () => {
    // Every field here is optional forever. A daemon older than `startedAt`
    // must still start a card, and a card with no timer is the intended
    // outcome — unlike a zero, which would count up from January 1970.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')

    const [, activity] = pushes(calls)
    expect(stateOf(activity)).toEqual({
      terminal: 'term-1',
      label: 'claude needs you',
      machine: 'Studio',
      status: 'blocked',
      detail: 'Waiting for your answer',
      blocked: 1,
      review: 0,
      working: 0,
      more: 0,
      rows: [row({ label: 'claude needs you' })],
    })
    expect('startedAt' in activity.body.aps['content-state']).toBe(false)
    // And absent on the row too, for the same reason: a row with no clock draws
    // no timer, where a zero would count up from January 1970.
    expect('startedAt' in activity.body.aps['content-state'].rows[0]).toBe(false)
  })

  it('never repeats the attributes on an update or an end', async () => {
    // Attributes are the activity's identity, fixed for its whole life, and
    // APNs REJECTS a push that repeats them. One leaking onto either of these
    // does not merely say something twice — it costs the update entirely, so the
    // card freezes on whatever it last said and looks like a dead relay.
    //
    // The turn clock rides the STATE now and therefore goes out on every push,
    // which is the point: a card that changes leader has to be able to change
    // the clock, and only the state can carry something that changes.
    const calls = watchFetch()
    await ready()
    await running('term-1')
    const clock = { startedAt: 1_755_000_000_000 }
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working', ...clock }, 'mine')
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'blocked', ...clock }, 'mine')
    await post('/v1/notify', { title: 'Done', terminal: 'term-1', status: 'done', ...clock }, 'mine')

    const events = pushes(calls)
      .filter(call => call.body.aps.event)
      .map(call => [call.body.aps.event, call.body.aps.attributes, call.body.aps['content-state']])
    expect(events.map(([event]) => event)).toEqual(['update', 'update', 'end'])
    for (const [event, attributes, state] of events) {
      expect(attributes).toBeUndefined()
      expect((state as any).startedAt).toBe(1_755_000_000_000)
      expect(event).not.toBe('start')
    }
  })

  it('updates the activity already running rather than starting a second', async () => {
    const calls = watchFetch()
    await ready()
    await running('term-1')
    await post(
      '/v1/notify',
      { title: 'hi', subtitle: 'Still waiting', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    const [, activity] = pushes(calls)
    expect(activity.url).toContain('/device/update-token')
    expect(activity.body.aps.event).toBe('update')
    // Attributes on anything but a start is an error — they describe the
    // activity's identity, which cannot change once it exists.
    expect(activity.body.aps['attributes-type']).toBeUndefined()
    expect(activity.body.aps.attributes).toBeUndefined()
  })

  it('starts nothing on a phone that has not offered a push-to-start token', async () => {
    const calls = watchFetch()
    await register('user_1')
    await pair('user_1', 'mine')
    await post('/v1/notify', { title: 'hi', terminal: 'term-1', status: 'blocked' }, 'mine')

    expect(pushes(calls).length).toBe(1)
  })

  it('ends the activity and forgets the token', async () => {
    const calls = watchFetch()
    await ready()
    await running('term-1')
    await post(
      '/v1/notify',
      { title: 'Done', subtitle: 'Tests pass', terminal: 'term-1', status: 'done' },
      'mine',
    )

    const [, activity] = pushes(calls)
    expect(activity.url).toContain('/device/update-token')
    expect(activity.body.aps.event).toBe('end')
    // The final state is shown briefly, then a dismissal date clears it —
    // without one the card sits on the lock screen for hours.
    //
    // The finished agent is still a ROW, in the to-review tier, because that is
    // what it is: `done` is a run waiting to be read. It is only that nothing is
    // left blocked or working that takes the card down.
    expect(stateOf(activity)).toEqual({
      terminal: 'term-1',
      label: 'Done',
      machine: 'Studio',
      status: 'done',
      detail: 'Tests pass',
      blocked: 0,
      review: 1,
      working: 0,
      more: 0,
      rows: [row({ label: 'Done', status: 'done', detail: 'Tests pass' })],
    })
    expect(activity.body.aps['dismissal-date']).toBeGreaterThan(1_600_000_000)
    expect(activity.body.aps.attributes).toBeUndefined()

    const rows = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(rows.results?.length).toBe(0)
  })

  it('does not push-to-start something that has already finished', async () => {
    // There is no activity to end, and starting one to announce it is over
    // leaves a card the user cannot dismiss.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'Done', terminal: 'term-1', status: 'done' }, 'mine')

    expect(pushes(calls).length).toBe(1)
  })

  it('delivers the alert even when the Live Activity push fails', async () => {
    // The alert is the guarantee; the activity is the enhancement. A dead
    // update token must not cost anyone the notification.
    const calls = watchFetch(call => {
      if (call.body?.aps?.event) throw new Error('BadDeviceToken')
      return ok()
    })
    await ready()
    await running('term-1')
    const response = await post(
      '/v1/notify',
      { title: 'hi', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    expect(response.status).toBe(200)
    expect(await response.json<{ delivered: number }>()).toEqual({ delivered: 1 })
    expect(pushes(calls).length).toBe(2)
  })

  it('does not touch an activity belonging to another account', async () => {
    // Same rule as the alert: the daemon names a terminal, never a destination.
    const calls = watchFetch()
    await ready()
    await env.DB.prepare(
      `INSERT INTO accounts (id, created_at) VALUES (?, ?) ON CONFLICT (id) DO NOTHING`,
    )
      .bind('user_2', Date.now())
      .run()
    await env.DB.prepare(
      `INSERT INTO install_cards
         (id, account_id, update_token, leader_terminal, leader_status, updated_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    )
      .bind(crypto.randomUUID(), 'user_2', 'their-update-token', 'term-1', 'blocked', Date.now())
      .run()

    await post('/v1/notify', { title: 'hi', terminal: 'term-1', status: 'blocked' }, 'mine')
    expect(calls.every(call => !call.url.includes('their-update-token'))).toBe(true)
  })

  it('refreshes a running card for a working agent and wakes nobody', async () => {
    // The distinction the whole working tier rests on. A card update is silent
    // and goes to a card the person already chose to watch; an alert is an
    // interruption. A working agent is the NORMAL case, so if it ever produced
    // the second one this feature would be the reason people turn the app's
    // notifications off — taking the blocked and done ones with them.
    const calls = watchFetch()
    await ready()
    await running('term-1')
    await post(
      '/v1/notify',
      { title: 'claude', subtitle: '3/7 · Designing test matrix', terminal: 'term-1', status: 'working' },
      'mine',
    )

    const sent = pushes(calls)
    expect(sent.length).toBe(1)
    expect(sent[0].headers['apns-push-type']).toBe('liveactivity')
    expect(sent[0].url).toContain('/device/update-token')
    expect(sent[0].body.aps.event).toBe('update')
    expect(stateOf(sent[0])).toEqual({
      terminal: 'term-1',
      label: 'claude',
      machine: 'Studio',
      status: 'working',
      detail: '3/7 · Designing test matrix',
      blocked: 0,
      review: 0,
      working: 1,
      more: 0,
      rows: [row({ status: 'working', detail: '3/7 · Designing test matrix' })],
    })
    // No alert dictionary: an activity push carrying one is PRESENTED, which is
    // the banner this tier must never produce.
    expect(sent[0].body.aps.alert).toBeUndefined()
    // Priority 10 spends the app's Live Activity budget, and it is the same
    // budget the blocked alert depends on.
    expect(sent[0].headers['apns-priority']).toBe('5')
  })

  it('starts NOTHING for an agent that has merely begun working', async () => {
    // This test asserted the opposite until 2026-09-06, and what it was
    // asserting never worked.
    //
    // A `working` start had to be silent — a banner every time any agent picked
    // up work is the notification people switch the app off over — and **iOS
    // discards a push-to-start activity that carries no alert dictionary.**
    // Silently, after APNs has already answered 200. So every one of these
    // starts was thrown away by the phone, and this suite could not see it:
    // `fetch` is mocked here, the relay sent exactly what it meant to send, and
    // the platform dropped it at the far end. That is why a card people were
    // meant to see on every run turned up two or three times in total.
    //
    // The card starts on `blocked` instead, which used to look expensive because
    // it meant losing the busy-agent card. With a row per agent it is not: once
    // the card exists it draws every agent, working ones included, so the only
    // case given up is work happening with nobody needed — which could never
    // have started silently anyway.
    const calls = watchFetch()
    await ready()
    await post(
      '/v1/notify',
      { title: 'claude', subtitle: 'Reading watch.rs', terminal: 'term-1', status: 'working' },
      'mine',
    )

    expect(pushes(calls)).toEqual([])
    // And no row claiming the install's one card slot, because no card was
    // raised. Claiming for a card that does not exist is what left this feature
    // permanently wedged.
    const cards = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(cards.results?.length).toBe(0)
  })

  it('starts a card that alerts, because a silent start is discarded', async () => {
    // The requirement, stated as the platform states it: iOS presents a
    // push-to-start activity or it drops it, and there is no third behavior. So
    // the question was never whether to alert — it was whether there is anything
    // worth alerting about at the moment a card starts.
    //
    // There is, and it is the fleet's own header. "2 need you · 3 in flight" is
    // a statement about the fleet that a person is entitled to be interrupted
    // by; "claude started working" is not, and that is exactly the banner this
    // would have been if the card still started on `working`.
    const calls = watchFetch()
    await ready()
    await post(
      '/v1/notify',
      { title: 'claude needs you', subtitle: 'Force-push to origin/main?', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    const [, activity] = pushes(calls)
    expect(activity.body.aps.event).toBe('start')
    expect(activity.body.aps.alert).toEqual({
      title: '1 needs you',
      body: 'Force-push to origin/main?',
    })
    // Priority 10 on a start, whatever its status. A start at 5 is one APNs may
    // throttle or hold, and the one push that has to arrive for the card to
    // exist at all was going out at the lower urgency.
    expect(activity.headers['apns-priority']).toBe('10')
  })

  it('gives the card it starts its turn clock', async () => {
    // A card without it counts nothing for as long as it exists. It travels in
    // the state rather than the attributes, so it can follow a change of
    // headline — but a card started without one still shows no elapsed time
    // until the next push arrives.
    const calls = watchFetch()
    await ready()
    await post(
      '/v1/notify',
      {
        title: 'claude needs you',
        subtitle: 'Writing fruit.txt',
        terminal: 'term-1',
        status: 'blocked',
        label: 'claude',
        startedAt: 1_755_000_000_000,
      },
      'mine',
    )

    const [, activity] = pushes(calls)
    expect(activity.body.aps['attributes-type']).toBe('AgentActivityAttributes')
    expect(activity.body.aps.attributes).toEqual({ version: 3 })
    expect(stateOf(activity)).toEqual({
      terminal: 'term-1',
      label: 'claude',
      machine: 'Studio',
      status: 'blocked',
      detail: 'Writing fruit.txt',
      startedAt: 1_755_000_000_000,
      blocked: 1,
      review: 0,
      working: 0,
      more: 0,
      rows: [row({ detail: 'Writing fruit.txt', startedAt: 1_755_000_000_000 })],
    })
    // A NUMBER, in milliseconds. A string reads back as nil in the app and
    // costs the timer without costing the card.
    expect(typeof activity.body.aps['content-state'].startedAt).toBe('number')
  })

  it('starts ONE card, however many blocked pushes arrive', async () => {
    // The failure this row exists to prevent, and the worst one this feature
    // could ship with. Only the app can report an update token, and the app may
    // never run — so without a row written when a card is raised, every push
    // that follows finds nothing running and starts ANOTHER card, none of which
    // the relay holds a token for and none of which it can ever take back.
    //
    // Blocked pushes rather than working ones, which is the whole shape of the
    // fix: a run no longer raises a card by being busy, so the stream this was
    // written against is now the much rarer one of an agent asking repeatedly.
    const calls = watchFetch()
    await ready()
    for (const detail of ['Force-push?', 'Force-push?', 'Force-push?']) {
      await post(
        '/v1/notify',
        { title: 'claude needs you', subtitle: detail, terminal: 'term-1', status: 'blocked' },
        'mine',
      )
    }

    const starts = pushes(calls).filter(call => call.body.aps?.event === 'start')
    expect(starts.length).toBe(1)
    expect(starts[0].url).toContain('/device/start-token')

    // One row, holding the sentinel: a card is running for this install and
    // nothing yet knows where. It is what the UNIQUE (account_id) constraint
    // refuses the second start against, and it remembers which agent the card is
    // headlining.
    const rows = await env.DB.prepare(
      `SELECT leader_terminal, leader_status, update_token FROM install_cards`,
    ).all<any>()
    expect(rows.results).toEqual([
      { leader_terminal: 'term-1', leader_status: 'blocked', update_token: '' },
    ])
  })

  it('updates the card it started blind once the app reports its token', async () => {
    // Recovery, and it needs nothing from anybody: iOS replays running
    // activities to the app at launch, the app files the update token for a card
    // it never started, and the row the relay wrote blind fills in. From that
    // moment the card is addressable for the rest of the run.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    await running('term-1')
    await post(
      '/v1/notify',
      { title: 'claude', subtitle: 'Running tests', terminal: 'term-1', status: 'working' },
      'mine',
    )

    const activities = pushes(calls).filter(call => call.body.aps?.event)
    const [start, update, ...rest] = activities
    expect(rest).toEqual([])
    expect(start.body.aps.event).toBe('start')
    expect(update.url).toContain('/device/update-token')
    expect(update.body.aps.event).toBe('update')
    expect(stateOf(update)).toEqual({
      terminal: 'term-1',
      label: 'claude',
      machine: 'Studio',
      status: 'working',
      detail: 'Running tests',
      blocked: 0,
      review: 0,
      working: 1,
      more: 0,
      rows: [row({ status: 'working', detail: 'Running tests' })],
    })
    // Still an update, so still no attributes — they are the activity's
    // identity and APNs rejects a push that repeats them.
    expect(update.body.aps.attributes).toBeUndefined()
  })

  it('alerts for a block on a card it cannot reach', async () => {
    // The tier that matters, on a card the relay cannot yet address. The alert
    // is the promise and it goes out untouched and first, before anything is
    // done about the card at all — see `pushActivity`'s placement in `notify`.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    await post(
      '/v1/notify',
      { title: 'claude needs you again', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    // Two alert pushes, one per block, whatever the card could or could not do
    // about itself in between.
    const alerts = pushes(calls).filter(call => call.headers['apns-push-type'] === 'alert')
    expect(alerts.length).toBe(2)
    expect(alerts[1].url).toContain('/device/device-token')
  })

  it('forgets a card it cannot address when the agent finishes', async () => {
    // There is nowhere to send the end — only the app could have told the relay
    // where — so it sends nothing rather than pushing at the empty string and
    // counting a delivery that cannot have happened. The row goes anyway: an
    // update token dies with its activity, and a row kept past the run would
    // refuse this install a card for every run after it. The abandoned card
    // clears itself on the `stale-date` its start carried.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    await post('/v1/notify', { title: 'Done', terminal: 'term-1', status: 'done' }, 'mine')

    const activities = pushes(calls).filter(call => call.body.aps?.event)
    expect(activities.map(call => call.body.aps.event)).toEqual(['start'])

    const rows = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(rows.results?.length).toBe(0)

    // And the next block gets its card, because the row it would have collided
    // with is gone.
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-2', status: 'blocked' }, 'mine')
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(2)
  })

  it('puts the fleet header on the alert its start carries', async () => {
    // The tier above working: a banner, at priority 10, and a card started from
    // the push-to-start token — and the start's own alert, which is not the
    // notice's title.
    //
    // "1 needs you" rather than "claude needs you" is the ruling, not a
    // simplification. iOS requires a start to alert, and what makes that
    // legitimate is that the thing being announced is the FLEET's state. The
    // agent's own sentence is already on the banner beside it, which is a
    // different push about a different thing.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')

    const [alert, activity] = pushes(calls)
    expect(alert.headers['apns-push-type']).toBe('alert')
    expect(alert.body.aps.alert).toEqual({ title: 'claude needs you', body: '' })
    expect(activity.body.aps.event).toBe('start')
    expect(activity.headers['apns-priority']).toBe('10')
    expect(activity.body.aps.alert).toEqual({ title: '1 needs you', body: '' })
  })

  it('never has a card to replace, because none starts on Working', async () => {
    // This used to be "replaces a card stuck on Working when the agent blocks",
    // and the branch it guarded is now deleted rather than merely unused.
    //
    // The scenario it described was real: a silent `working` start claimed the
    // row, the `blocked` push that followed found a card it could not address,
    // and the lock screen read "Working" beside a banner saying the agent needed
    // an answer. A second card was started to say the true thing. Nothing starts
    // on `working` any more, and `startCard` records the headline it started
    // with — always the blocked agent, because blocked sorts first — so a blind
    // card is already showing the only tier that can raise one. There is nothing
    // left to correct, and a second card would be a duplicate for no gain.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
    await post(
      '/v1/notify',
      { title: 'claude needs you', subtitle: 'Create haiku.txt?', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    const starts = pushes(calls).filter(call => call.body.aps?.event === 'start')
    expect(starts.length).toBe(1)
    // And the working agent is not lost: it is a ROW on the card the block
    // raised, which is the whole reason giving up the working start costs
    // nothing. Here it is the same agent, so there is one row and it now reads
    // blocked.
    expect(stateOf(starts[0])).toEqual({
      terminal: 'term-1',
      label: 'claude needs you',
      machine: 'Studio',
      status: 'blocked',
      detail: 'Create haiku.txt?',
      blocked: 1,
      review: 0,
      working: 0,
      more: 0,
      rows: [row({ label: 'claude needs you', detail: 'Create haiku.txt?' })],
    })
    // The row remembers the headline it is showing.
    const card = await env.DB.prepare(
      `SELECT leader_terminal, leader_status FROM install_cards`,
    ).first<any>()
    expect(card?.leader_terminal).toBe('term-1')
    expect(card?.leader_status).toBe('blocked')
  })

  it('starts one card however many times the agent blocks', async () => {
    // Without the row, every blocked push while unaddressable would stack
    // another card — the same failure `TOKEN_UNKNOWN` was written to prevent.
    const calls = watchFetch()
    await ready()
    for (const question of ['Create haiku.txt?', 'Delete build/?', 'Force push?']) {
      await post(
        '/v1/notify',
        { title: 'claude needs you', subtitle: question, terminal: 'term-1', status: 'blocked' },
        'mine',
      )
    }

    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(1)
  })

  it('does not raise a card the person has just swiped away', async () => {
    // The undismissable card. A working push arrives about every ten seconds, so
    // a dismissal the relay forgot was a card back on the lock screen before the
    // phone was back in a pocket — for the length of the run.
    const calls = watchFetch()
    await ready()
    await running('term-1')
    await post(
      '/v1/devices/activity',
      { terminal: 'term-1', updateToken: null, dismissed: true },
      await sessionFor('user_1'),
    )
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')

    expect(pushes(calls).filter(call => call.body.aps?.event).length).toBe(0)
    // The row outlives the card on purpose: it is the refusal being remembered.
    const row = await env.DB.prepare(`SELECT update_token, dismissed_at FROM install_cards`)
      .first<any>()
    expect(row?.update_token).toBe('')
    expect(row?.dismissed_at).toBeGreaterThan(0)
  })

  it('still raises one when the dismissed agent goes on to block', async () => {
    // A dismissal is a refusal of what the card was saying, not of everything
    // this agent will ever say. Blocking is news the person has not seen.
    const calls = watchFetch()
    await ready()
    await running('term-1')
    await post(
      '/v1/devices/activity',
      { terminal: 'term-1', updateToken: null, dismissed: true },
      await sessionFor('user_1'),
    )
    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    const starts = pushes(calls).filter(call => call.body.aps?.event === 'start')
    expect(starts.length).toBe(1)
    expect(starts[0].body.aps['content-state'].status).toBe('blocked')
  })

  it('clears the dismissal it supersedes, so one swipe costs exactly one card', async () => {
    // `startCard`'s conflict arm, which nothing reached. The test above proves a
    // card goes up after a dismissal; what it never asked was what the arm wrote
    // to the row on the way — so `DO UPDATE SET … dismissed_at = NULL …` could
    // be replaced with `DO NOTHING` and the suite stayed green.
    //
    // The consequence is the one the dismissal was written to prevent. A swipe
    // that is never cleared is a swipe that is answered again by every blocked
    // push that follows: a second card, then a third, for an agent that has
    // asked one question. The clearing is what makes it one card per dismissal
    // rather than one per notice.
    const calls = watchFetch()
    const session = await sessionFor('user_1')
    await ready()
    await running('term-1')
    await post('/v1/devices/activity', { updateToken: null, dismissed: true }, session)

    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(1)

    // The row now stands for the card that just went up: the refusal is spent,
    // and the headline is the one the start is showing.
    const card = await cardOf('user_1')
    expect(card?.dismissed_at).toBe(null)
    expect(card?.leader_terminal).toBe('term-1')
    expect(card?.leader_status).toBe('blocked')

    // So the next blocked push finds nothing dismissed and falls through, which
    // is the behavior the clearing exists for.
    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(1)
  })

  it('leaves alone a row the app re-addressed while the start was in flight', async () => {
    // The `WHERE` on that arm. The app files a real update token while the start
    // pushes are still going out — which is precisely what happens when the
    // phone comes to the foreground because of the alert they carry — and the
    // row it files against is no longer the one this start claimed. The arm
    // declines it, so what the app wrote stands and neither of the row's two
    // clocks is re-stamped by a start that filing has already overtaken.
    //
    // The race is reproduced by filing the token from inside the reply to the
    // start push, which is the only place it can be made to happen on purpose.
    const session = await sessionFor('user_1')
    let filed = false
    watchFetch(async call => {
      if (call.body?.aps?.event === 'start' && !filed) {
        filed = true
        await post('/v1/devices/activity', { updateToken: 'filed-by-the-app' }, session)
      }
      return ok()
    })
    await ready()
    await running('term-9')
    await post('/v1/notify', { title: 'codex', terminal: 'term-9', status: 'working' }, 'mine')
    await post('/v1/devices/activity', { updateToken: null, dismissed: true }, session)
    const dismissed = await cardOf('user_1')

    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    const card = await cardOf('user_1')
    expect(filed).toBe(true)
    expect(card?.update_token).toBe('filed-by-the-app')
    // Untouched by the start: this is the app's row now, and the next notice
    // through the update branch is what re-composes it.
    expect(card?.leader_terminal).toBe('term-9')
    expect(card?.leader_status).toBe('working')
    expect(card?.pushed_at).toBe(dismissed?.pushed_at ?? null)
  })

  it('forgets a claim that has outlived the card it stands for', async () => {
    // `CLAIM_MEMORY_MS`, and the read that `install_cards.updated_at` never had.
    //
    // The column was written in four places and read in none, so a row standing
    // for a card that never appeared — a start iOS discarded, a phone whose app
    // is never opened — held this install's only slot for good. The relay
    // believed in a card nobody could see and refused, silently and
    // permanently, to start another.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(1)

    // A second block, right away, correctly starts nothing: the claim is fresh
    // and the card it stands for is presumed to be up.
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(1)

    // An hour on, the relay no longer knows whether that card is on any lock
    // screen, and the chance of a fresh one is worth more than the claim.
    await env.DB.prepare(`UPDATE install_cards SET updated_at = ?`)
      .bind(Date.now() - 2 * 60 * 60 * 1000)
      .run()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(2)
  })

  it('forgets the card outright when it merely ended', async () => {
    // Not every ending is a refusal. The relay's own `end`, or a card iOS
    // retired, leaves nothing to remember — and remembering it would silence the
    // next run's card for an hour.
    watchFetch()
    await ready()
    await running('term-1')
    await post(
      '/v1/devices/activity',
      { terminal: 'term-1', updateToken: null },
      await sessionFor('user_1'),
    )

    const rows = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(rows.results?.length).toBe(0)
  })

  it('clears the memory of a dismissal once the app files a real token', async () => {
    // A real update token means there is a card, it is up, and it can be moved
    // in place. Anything the relay remembered about not being able to reach one
    // is answered by that.
    //
    // The LEADER survives, and that is the half that changed. `blind_status` was
    // cleared here because it only ever meant "what the card the relay cannot
    // reach is showing"; `leader_status` means what the card is showing whether
    // or not it can be reached, and clearing it would make every card forget its
    // leader the moment the app came to the foreground.
    watchFetch()
    const session = await sessionFor('user_1')
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    await running('term-1')
    await post('/v1/devices/activity', { terminal: 'term-1', updateToken: null, dismissed: true }, session)
    await running('term-1', 'fresh-token')

    const card = await env.DB.prepare(
      `SELECT update_token, leader_terminal, leader_status, dismissed_at FROM install_cards`,
    ).first<any>()
    expect(card?.update_token).toBe('fresh-token')
    expect(card?.leader_terminal).toBe('term-1')
    expect(card?.leader_status).toBe('blocked')
    expect(card?.dismissed_at).toBe(null)
  })

  it('does not claim the card slot for a start APNs refused', async () => {
    // `startCard` wrote its row BEFORE pushing, and that ordering is why this
    // feature could wedge permanently. A start APNs rejected — or, until this
    // commit, every start, because none carried the alert iOS requires — still
    // left a row holding this install's only slot with `update_token = ''`. The
    // relay then believed in a card nobody could see, and `UNIQUE (account_id)`
    // refused every start that followed, for good.
    //
    // The claim goes after an accepted push now. What made that safe is the
    // other half of the ruling: `working` no longer starts cards, so the stream
    // of ten-second retries the pre-claim was defending against does not exist.
    const calls = watchFetch(() => new Response('{"reason":"BadDeviceToken"}', { status: 400 }))
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')

    // It tried, and APNs said no.
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(1)
    const cards = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(cards.results?.length).toBe(0)
  })

  it('sends a start at priority 10 whatever tier its state names', async () => {
    // Tested against the transport rather than through a route, and that is the
    // point: a card only starts on `blocked` now, so a start whose state says
    // `working` cannot be produced by `/v1/notify` — and a test that went in
    // that way would be green against the defect. It would prove that a blocked
    // start is priority 10, which the OLD rule already said.
    //
    // The rule this pins is the one that was wrong: a start is never routine,
    // whatever its status. A priority-5 push is one APNs may throttle or hold,
    // and the one push that has to arrive for the card to exist at all was
    // going out at the lower urgency for the entire life of the silent
    // `working` start.
    const calls = watchFetch()
    const state = { terminal: 't', label: 'a', machine: 'm', status: 'working' as const, detail: '' }
    await sendLiveActivity(
      env,
      'start-token',
      { event: 'start', state, alert: { title: '1 needs you', body: '' }, attributes: { version: 3 } },
      null,
    )

    expect(pushes(calls)[0].headers['apns-priority']).toBe('10')
  })

  it('refuses to send a start with no alert, because iOS would discard it', async () => {
    // The check the platform will not give us. iOS drops a push-to-start
    // activity that carries no alert dictionary, silently, after APNs has
    // already answered 200 — so this suite, which mocks `fetch` and asserts the
    // relay sent what it meant to send, could never have caught it. Refusing at
    // the transport is what makes the failure say something.
    const calls = watchFetch()
    const state = { terminal: 't', label: 'a', machine: 'm', status: 'blocked' as const, detail: '' }
    const sent = await sendLiveActivity(
      env,
      'start-token',
      { event: 'start', state, attributes: { version: 3 } },
      null,
    )

    expect(sent).toBe(false)
    expect(pushes(calls)).toEqual([])
  })

  it('leaves activities alone when the daemon names no terminal', async () => {
    // A card leads with one terminal and puts it in the URL a tap opens, and a
    // leader under the empty string is a card that cannot say which agent it is
    // about and cannot be retired by the runner that knows. Better no card at
    // all.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'hi', status: 'blocked' }, 'mine')

    expect(pushes(calls).length).toBe(1)
  })

  // MARK: - One card, several agents

  /// What one card per install actually costs and buys, agent by agent.
  ///
  /// Everything above this point tests one terminal, which is the case that was
  /// already right. The rules below are the ones the rekey added, and every one
  /// of them exists because the relay now has to CHOOSE on every push: it holds
  /// one row, four agents push into it, and nothing in this worker survives
  /// between requests except those two columns.
  describe('with several agents on one card', () => {
    it('keeps one card however many agents block', async () => {
      // The failure this whole change exists to remove. Four agents used to mean
      // four cards stacked on the lock screen, and a Dynamic Island that can
      // present exactly one picking between them with no rule anybody wrote.
      const calls = watchFetch()
      await ready()
      for (const terminal of ['term-1', 'term-2', 'term-3', 'term-4']) {
        await post('/v1/notify', { title: 'claude needs you', terminal, status: 'blocked' }, 'mine')
      }

      const starts = pushes(calls).filter(call => call.body.aps?.event === 'start')
      expect(starts.length).toBe(1)
      expect(starts[0].body.aps['content-state'].terminal).toBe('term-1')

      const cards = await env.DB.prepare(`SELECT leader_terminal FROM install_cards`).all<any>()
      expect(cards.results).toEqual([{ leader_terminal: 'term-1' }])
    })

    it('gives a working agent that is not the headline a ROW instead of silence', async () => {
      // **This test asserted the opposite, and the opposite was the bug.**
      //
      // `leads` was a GATE: an agent that did not hold the card pushed nothing
      // at all, so four busy agents behind one stuck one were invisible, and a
      // wedged leader silenced the whole fleet. It was gating for a real reason
      // — four agents each taking the card would flip it six times a minute and
      // spend the Live Activity budget the blocked alert depends on — but the
      // remedy was to drop three quarters of the fleet.
      //
      // With a row each there is nothing to flip: the card names every agent and
      // the headline is a sort, not a claim. The budget is still real and is
      // still defended, one layer down, by `COALESCE_MS`.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
      await post('/v1/notify', { title: 'codex', terminal: 'term-2', status: 'working', label: 'codex' }, 'mine')

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      // The blocked agent still headlines — that precedence did not change.
      expect(last.terminal).toBe('term-1')
      // And the busy one is on the card, which is what it never was before.
      expect(last.rows.map((each: any) => each.terminal)).toEqual(['term-1', 'term-2'])
      expect(last.blocked).toBe(1)
      expect(last.working).toBe(1)
      const card = await env.DB.prepare(`SELECT leader_terminal FROM install_cards`).first<any>()
      expect(card?.leader_terminal).toBe('term-1')
    })

    it('headlines an agent that blocks while the others merely work', async () => {
      // Blocked outranks working, always. An agent waiting on a person is the
      // one thing the lock screen exists to show, and a busy agent must never
      // hold the top line against it.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
      await post(
        '/v1/notify',
        { title: 'codex needs you', subtitle: 'Run cargo test?', terminal: 'term-2', status: 'blocked', label: 'codex' },
        'mine',
      )

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      expect(stateOf(updates[updates.length - 1])).toEqual({
        terminal: 'term-2',
        label: 'codex',
        machine: 'Studio',
        status: 'blocked',
        detail: 'Run cargo test?',
        blocked: 1,
        review: 0,
        working: 1,
        more: 0,
        rows: [
          row({ terminal: 'term-2', label: 'codex', detail: 'Run cargo test?' }),
          row({ terminal: 'term-1', status: 'working', detail: '' }),
        ],
      })
      const card = await env.DB.prepare(
        `SELECT leader_terminal, leader_status FROM install_cards`,
      ).first<any>()
      expect(card?.leader_terminal).toBe('term-2')
      expect(card?.leader_status).toBe('blocked')
    })

    it('never lets a working agent take the headline from a blocked one', async () => {
      // The refusal that matters most, and the half of `leads` that survives.
      // The card reads "Needs You" and an agent three panes over picks up work;
      // moving the top line would replace the one notification this product
      // exists to deliver with a progress line. The busy agent gets a row under
      // it instead, which is the whole point.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
      await post('/v1/notify', { title: 'codex', terminal: 'term-2', status: 'working' }, 'mine')

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      expect(updates[updates.length - 1].body.aps['content-state'].terminal).toBe('term-1')
      const card = await env.DB.prepare(`SELECT leader_terminal FROM install_cards`).first<any>()
      expect(card?.leader_terminal).toBe('term-1')
    })

    it('keeps the card with whichever agent blocked first', async () => {
      // The one that has been waiting longest keeps it. Swapping the leader on
      // each new question would rewrite the card out from under somebody in the
      // middle of reading it — and the second agent's ALERT still fires, because
      // that is a separate push decided before the card is touched at all.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
      await post('/v1/notify', { title: 'codex needs you', terminal: 'term-2', status: 'blocked' }, 'mine')

      const alerts = pushes(calls).filter(call => call.headers['apns-push-type'] === 'alert')
      expect(alerts.length).toBe(2)
      const row = await env.DB.prepare(`SELECT leader_terminal FROM install_cards`).first<any>()
      expect(row?.leader_terminal).toBe('term-1')
    })

    it('lets the leader answer its own question and go back to working', async () => {
      // A card's own agent always lands, whatever the tier — which is how a
      // blocked leader stops being one. Without that the first question of a run
      // would pin the card for the rest of it.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
      await post(
        '/v1/notify',
        { title: 'claude', subtitle: 'Running tests', terminal: 'term-1', status: 'working' },
        'mine',
      )

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      expect(updates[updates.length - 1].body.aps['content-state'].status).toBe('working')
      const row = await env.DB.prepare(`SELECT leader_status FROM install_cards`).first<any>()
      expect(row?.leader_status).toBe('working')
    })

    it('does not end the card when an agent it is not about finishes', async () => {
      // With a card per terminal every `done` ended its own card. With one card
      // it would end SOMEBODY ELSE'S — clearing the lock screen of a running
      // agent because a different one stopped. The alert for that agent still
      // goes out; only the card is left alone.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
      await post('/v1/notify', { title: 'codex finished', terminal: 'term-2', status: 'done' }, 'mine')

      const ends = pushes(calls).filter(call => call.body.aps?.event === 'end')
      expect(ends.length).toBe(0)
      const alerts = pushes(calls).filter(call => call.headers['apns-push-type'] === 'alert')
      expect(alerts.length).toBe(2)
      const rows = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
      expect(rows.results?.length).toBe(1)
    })

    it('keeps the card up when the headline finishes and others are still going', async () => {
      // **The cost of one card, and it is no longer paid.**
      //
      // This test asserted that the leader finishing ENDED the card, while other
      // agents were still running, and that the next working push started a
      // fresh one. That was the honest consequence of a card that could only be
      // about one agent — but it cleared the lock screen of three running agents
      // because a fourth stopped, and the replacement card was a second one
      // beside the finished one for a minute.
      //
      // A card is about the fleet now. An agent finishing is a row changing
      // tier: it leaves "in flight" and joins "to review", and the card is still
      // true about everybody else.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
      await post('/v1/notify', { title: 'codex', terminal: 'term-2', status: 'working' }, 'mine')
      await post('/v1/notify', { title: 'claude finished', terminal: 'term-1', status: 'done' }, 'mine')

      expect(pushes(calls).filter(call => call.body.aps?.event === 'end').length).toBe(0)
      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      expect(last.review).toBe(1)
      expect(last.working).toBe(1)
      // Still one card, still addressable.
      expect((await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()).results?.length).toBe(1)
    })

    it('ends the card only when the last agent stops', async () => {
      // And it does end. A fleet with nothing blocked and nothing working has
      // nothing the card is for, and a card left up would read "Finished" over
      // an empty fleet until iOS expired it. The last state stays for
      // `DISMISSAL_DELAY_S`, which is what the finished state is FOR: somebody
      // who picks the phone up because of the alert has a last word to read.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
      await post('/v1/notify', { title: 'codex', terminal: 'term-2', status: 'working' }, 'mine')
      await post('/v1/notify', { title: 'claude finished', terminal: 'term-1', status: 'done' }, 'mine')
      await post('/v1/notify', { title: 'codex finished', terminal: 'term-2', status: 'done' }, 'mine')

      const ends = pushes(calls).filter(call => call.body.aps?.event === 'end')
      expect(ends.length).toBe(1)
      expect(ends[0].body.aps['dismissal-date']).toBeGreaterThan(ends[0].body.aps.timestamp)
      // Both agents are still ROWS on that last card, in the to-review tier.
      // They have not left the fleet; they have stopped needing a lock screen.
      expect(ends[0].body.aps['content-state'].review).toBe(2)
      expect((await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()).results?.length).toBe(0)
    })

    it('draws four rows and counts the rest', async () => {
      // `ROWS_SHOWN`. The header counts EVERY agent and the lines are the first
      // few, which is what `+N more` is for: a header that counted only what fit
      // would say "2 need you" while three agents were waiting.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      for (const terminal of ['t1', 't2', 't3', 't4', 't5', 't6', 't7']) {
        await post('/v1/notify', { title: 'claude', terminal, status: 'working' }, 'mine')
      }

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      expect(last.rows.length).toBe(4)
      expect(last.working).toBe(7)
      expect(last.more).toBe(3)
    })

    it('orders the rows blocked, then to review, then working', async () => {
      // The same precedence the rest of the product uses, and it means the row
      // that needs a person never falls off the bottom of a card with four
      // lines and seven agents.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'a', terminal: 'busy', status: 'working' }, 'mine')
      await post('/v1/notify', { title: 'b', terminal: 'read-me', status: 'done' }, 'mine')
      await post('/v1/notify', { title: 'c', terminal: 'stuck', status: 'blocked' }, 'mine')

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      expect(last.rows.map((each: any) => each.terminal)).toEqual(['stuck', 'read-me', 'busy'])
      expect(last.terminal).toBe('stuck')
    })

    it('headlines the agent that has been waiting longest, not the one that spoke last', async () => {
      // Within a tier, the longest-waiting row goes first. `status_since` is the
      // key and it is deliberately NOT `updated_at`: a blocked agent asks once
      // and goes quiet, while a busy one pushes every ten seconds, so ordering
      // by who spoke last would hand the top line to whoever is chattiest.
      //
      // It also has to survive being spoken to. `zeno` blocks, waits, and then
      // pushes `blocked` again — the same tier, so its place in the queue must
      // not move. That is what `status_since` moving only on a real tier change
      // buys, and nothing else in this suite would notice it stop.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'zeno needs you', terminal: 'zeno', status: 'blocked' }, 'mine')
      await env.DB.prepare(
        `UPDATE live_activities SET status_since = ? WHERE terminal = 'zeno'`,
      )
        .bind(Date.now() - 30 * 60 * 1000)
        .run()
      await post('/v1/notify', { title: 'aria needs you', terminal: 'aria', status: 'blocked' }, 'mine')
      // And now the older one speaks again, in the same tier.
      await post('/v1/notify', { title: 'zeno needs you', terminal: 'zeno', status: 'blocked' }, 'mine')

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      // `aria` sorts before `zeno` alphabetically, so the tiebreak cannot be
      // what produces this order — only the wait can.
      expect(last.rows.map((each: any) => each.terminal)).toEqual(['zeno', 'aria'])
      expect(last.terminal).toBe('zeno')
    })

    it('restamps a row the moment its tier moves, so a long run is not a long wait', async () => {
      // The other half of `status_since`, and the half nothing held. The test
      // above pins it STAYING PUT within a tier; making it never move at all —
      // `prior ? (prior.status_since ?? now) : now` — left the whole suite
      // green.
      //
      // What that costs is the top line of the card. `status_since` is "how long
      // this agent has been in the tier it is in", and an agent that has been
      // WORKING for an hour has been waiting for nothing. Carrying its old stamp
      // through the change puts it straight to the front of the blocked queue,
      // ahead of an agent that really has been waiting — so the person taps the
      // question that arrived last instead of the one that has been open
      // longest, and the longer the run the worse the placement.
      const calls = watchFetch()
      await ready()
      await running('term-1')

      // `aria` has been working for an hour.
      await post('/v1/notify', { title: 'aria', terminal: 'aria', status: 'working' }, 'mine')
      const hourAgo = Date.now() - 60 * 60 * 1000
      await env.DB.prepare(
        `UPDATE live_activities SET status_since = ?, updated_at = ? WHERE terminal = 'aria'`,
      )
        .bind(hourAgo, hourAgo)
        .run()

      // `zeno` has been blocked for ten minutes, which is a real wait.
      await post('/v1/notify', { title: 'zeno needs you', terminal: 'zeno', status: 'blocked' }, 'mine')
      await env.DB.prepare(
        `UPDATE live_activities SET status_since = ? WHERE terminal = 'zeno'`,
      )
        .bind(Date.now() - 10 * 60 * 1000)
        .run()

      // And now `aria` asks its first question.
      await post('/v1/notify', { title: 'aria needs you', terminal: 'aria', status: 'blocked' }, 'mine')

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      // `aria` sorts first alphabetically and spoke last, so neither the
      // tiebreak nor recency can be what produces this order.
      expect(last.rows.map((each: any) => each.terminal)).toEqual(['zeno', 'aria'])
      expect(last.terminal).toBe('zeno')

      // And the column says it directly. A tier change stamps both clocks
      // together; a notice within a tier moves only `updated_at`.
      const moved = await env.DB.prepare(
        `SELECT status_since, updated_at FROM live_activities WHERE terminal = 'aria'`,
      ).first<{ status_since: number; updated_at: number }>()
      expect(moved?.status_since).toBeGreaterThan(hourAgo)
      expect(moved?.status_since).toBe(moved?.updated_at)
    })

    it('starts the clock at a first sighting, whatever tier it arrives in', async () => {
      // A row with no `prior` has not been anywhere else, so the tier it arrives
      // in is the tier it has always been in and the stamp is now. NULL would
      // sort it to the front of its tier through `?? updated_at` — an agent the
      // relay has never heard of jumping ahead of one that has been waiting half
      // an hour.
      watchFetch()
      await ready()
      await running('term-1')
      const before = Date.now()
      await post('/v1/notify', { title: 'aria needs you', terminal: 'aria', status: 'blocked' }, 'mine')

      const row = await env.DB.prepare(
        `SELECT status_since FROM live_activities WHERE terminal = 'aria'`,
      ).first<{ status_since: number | null }>()
      expect(row?.status_since).not.toBe(null)
      expect(row!.status_since!).toBeGreaterThanOrEqual(before)
    })

    it('totals what the whole fleet changed, and says nothing for what nobody measured', async () => {
      // `+391 −112` under `+N more`, summed over every row rather than the ones
      // that fit. Derived on every push and stored nowhere: a stored copy is a
      // second source two writers can disagree about, which is the argument that
      // deleted `line` from this contract.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post(
        '/v1/notify',
        { title: 'a', terminal: 't1', status: 'working', insertions: 142, deletions: 37, commits: 4 },
        'mine',
      )
      await post(
        '/v1/notify',
        { title: 'b', terminal: 't2', status: 'blocked', insertions: 249, deletions: 75, commits: 1 },
        'mine',
      )
      // And one that has measured nothing: a worktree the runner has not probed,
      // or one with no base to compare against. It must not read as zero.
      await post('/v1/notify', { title: 'c', terminal: 't3', status: 'working' }, 'mine')

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      expect(last.insertions).toBe(391)
      expect(last.deletions).toBe(112)
      expect(last.commits).toBe(5)
      const unmeasured = last.rows.find((each: any) => each.terminal === 't3')
      expect('insertions' in unmeasured).toBe(false)
      expect('deletions' in unmeasured).toBe(false)
    })

    it('carries each row its own trace, and keeps the last one it was told', async () => {
      // Thirteen buckets per row, as the wire's base64, opaque all the way
      // through: the relay stores the string and the widget decodes it, and a
      // relay that parsed it would be a third copy of an encoding that already
      // has two ends.
      const calls = watchFetch()
      const trace = 'EQ'.repeat(44)
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'a', terminal: 't1', status: 'blocked', trace }, 'mine')
      // A later notice with no trace has not un-measured the history; it has no
      // new answer, and the row keeps the last real one.
      await post('/v1/notify', { title: 'a', terminal: 't1', status: 'blocked' }, 'mine')

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      expect(updates[updates.length - 1].body.aps['content-state'].rows[0].trace).toBe(trace)

      // And on the row, which is the assertion that actually guards the SQL.
      //
      // The card above is composed from what this request just supplied plus
      // what the previous one left, held in memory — so it agrees with the
      // carry-forward whether or not the UPSERT does. Only reading the column
      // back can tell a `COALESCE` from an assignment, and a third notice
      // arriving after this worker forgets everything would read exactly this.
      const stored = await env.DB.prepare(
        `SELECT trace, insertions FROM live_activities WHERE terminal = 't1'`,
      ).first<any>()
      expect(stored?.trace).toBe(trace)

      // And a caller cannot make the column any size it likes. `STATE_BUDGET`
      // would keep an oversized trace off the payload by dropping the row it is
      // on, which is a card silently missing an agent; the bound at the column
      // is what stops it becoming one.
      await post(
        '/v1/notify',
        { title: 'a', terminal: 't2', status: 'blocked', trace: 'Z'.repeat(4000) },
        'mine',
      )
      const fat = await env.DB.prepare(
        `SELECT trace FROM live_activities WHERE terminal = 't2'`,
      ).first<any>()
      expect(fat?.trace.length).toBe(128)
      // The same rule on the counts, which carry forward through the same
      // `COALESCE` and would otherwise be erased by a tick that measured
      // nothing.
      expect(stored?.insertions).toBe(null)
    })

    it('keeps a count on the row when a later notice measures nothing', async () => {
      // Absent means "no new answer", not "un-measured". A `working` tick that
      // arrives while the runner has not re-probed the worktree must not blank
      // the numbers the last one found — the row would draw nothing where it
      // drew `+142 −37` a second ago, and it would flicker back on the next
      // probe.
      watchFetch()
      await ready()
      await running('term-1')
      await post(
        '/v1/notify',
        { title: 'a', terminal: 't1', status: 'blocked', insertions: 142, deletions: 37, commits: 4 },
        'mine',
      )
      await post('/v1/notify', { title: 'a', terminal: 't1', status: 'blocked' }, 'mine')

      const stored = await env.DB.prepare(
        `SELECT insertions, deletions, commits FROM live_activities WHERE terminal = 't1'`,
      ).first<any>()
      expect(stored).toEqual({ insertions: 142, deletions: 37, commits: 4 })
    })

    it('holds a push that is only about volume, and never holds a tier change', async () => {
      // `COALESCE_MS`. A fleet card changes whenever any agent changes, which is
      // strictly more updates than a card about one agent was — and the budget
      // `leads` was protecting by refusing three agents in four does not
      // disappear when rows arrive, it moves here.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'a', terminal: 't1', status: 'working', insertions: 10 }, 'mine')
      const after = pushes(calls).filter(call => call.body.aps?.event === 'update').length

      // Volume moved and nothing else did, twice, inside ten seconds.
      await post('/v1/notify', { title: 'a', terminal: 't1', status: 'working', insertions: 20 }, 'mine')
      await post('/v1/notify', { title: 'a', terminal: 't1', status: 'working', insertions: 30 }, 'mine')
      expect(pushes(calls).filter(call => call.body.aps?.event === 'update').length).toBe(after)

      // The numbers were still STORED while the push was held, which is the
      // whole trade: the card is ten seconds behind, never wrong.
      const stored = await env.DB.prepare(
        `SELECT insertions FROM live_activities WHERE terminal = 't1'`,
      ).first<any>()
      expect(stored?.insertions).toBe(30)

      // And a tier change goes at once, whatever the clock says.
      await post('/v1/notify', { title: 'a needs you', terminal: 't1', status: 'blocked' }, 'mine')
      expect(pushes(calls).filter(call => call.body.aps?.event === 'update').length).toBe(after + 1)
    })

    it('forgets a row that has had nothing to say for a day', async () => {
      // `ROW_RETENTION_MS`, applied lazily on write because there are no cron
      // triggers in this relay. Twenty-four hours is the design's own widest
      // trace window: past it a row cannot contribute to anything the card can
      // draw, so it has nothing left to say.
      //
      // **Per ACCOUNT, and that half was guarded by nothing.** The purge is a
      // `DELETE` this account's own notice runs, so it is the one write in the
      // service that reaches every row in the table if its scoping goes: one
      // busy account would forget every other account's history on its way past.
      // With a single account's rows in the table the clause made no difference
      // to any assertion here, so the second account below is what makes it
      // observable — equally old, equally quiet, and none of this account's
      // business.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'old', terminal: 'yesterday', status: 'working' }, 'mine')
      await env.DB.prepare(
        `UPDATE live_activities SET updated_at = ? WHERE terminal = 'yesterday' AND account_id = ?`,
      )
        .bind(Date.now() - 25 * 60 * 60 * 1000, 'user_1')
        .run()
      await foreignAgent('user_2', 'their-yesterday', {
        status: 'working',
        updatedAt: Date.now() - 25 * 60 * 60 * 1000,
      })
      await post('/v1/notify', { title: 'new', terminal: 'today', status: 'working' }, 'mine')

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      expect(last.rows.map((each: any) => each.terminal)).toEqual(['today'])
      expect(last.working).toBe(1)
      expect(last.more).toBe(0)
      expect(await roster('user_1')).toEqual(['today'])
      // Somebody else's day-old row is still somebody else's.
      expect(await roster('user_2')).toEqual(['their-yesterday'])
    })

    it('draws and counts this account\'s fleet, and never another account\'s', async () => {
      // The read half of the account scoping, and the half with a lock screen
      // on the end of it. `readFleet` is where every row a card is composed
      // from comes from — the lines, the header's three counts, the totals and
      // `+N more` — so a `SELECT` that lost its `WHERE` would put a stranger's
      // agents, their runner names and their composed question, on this
      // person's phone.
      //
      // Nothing here could see that. Every other test in this file has exactly
      // one account's rows in `live_activities`, and against one account's rows
      // a scoped read and an unscoped read return the same thing. The second
      // account is what makes the clause load-bearing in a test rather than
      // only in a review.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      // Two of theirs, in two different tiers, each carrying counts — so an
      // unscoped read shows up in the lines, in the header AND in the totals
      // rather than in only one of the three.
      await foreignAgent('user_2', 'theirs-blocked', { status: 'blocked' })
      await foreignAgent('user_2', 'theirs-working', { status: 'working' })

      await post(
        '/v1/notify',
        {
          title: 'claude needs you',
          terminal: 'term-1',
          status: 'blocked',
          insertions: 5,
          deletions: 1,
          commits: 4,
        },
        'mine',
      )

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      expect(last.rows.map((each: any) => each.terminal)).toEqual(['term-1'])
      expect(last.blocked).toBe(1)
      expect(last.working).toBe(0)
      expect(last.review).toBe(0)
      expect(last.more).toBe(0)
      // The totals are summed over EVERY row the read returned, not only the
      // ones that got a line, so they are the assertion an unscoped read cannot
      // survive even when `ROWS_SHOWN` would have hidden the extra lines.
      expect(last.insertions).toBe(5)
      expect(last.deletions).toBe(1)
      expect(last.commits).toBe(4)
      // And their rows are still theirs afterwards.
      expect(await roster('user_2')).toEqual(['theirs-blocked', 'theirs-working'])
    })

    it('drops a quiet row to the tail without dropping it from the count', async () => {
      // `ROW_QUIET_AFTER_MS`, deliberately the same hour as `STALE_AFTER_S`: a
      // row stops spending a line exactly when the card as a whole would be
      // marked out of date. It is still IN the fleet — still counted in the
      // header and the totals — it has just stopped earning one of the few lines
      // the card has.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      await post('/v1/notify', { title: 'quiet', terminal: 'quiet', status: 'working' }, 'mine')
      await env.DB.prepare(`UPDATE live_activities SET updated_at = ? WHERE terminal = 'quiet'`)
        .bind(Date.now() - 2 * 60 * 60 * 1000)
        .run()
      await post('/v1/notify', { title: 'loud', terminal: 'loud', status: 'working' }, 'mine')

      const updates = pushes(calls).filter(call => call.body.aps?.event === 'update')
      const last = updates[updates.length - 1].body.aps['content-state']
      expect(last.rows.map((each: any) => each.terminal)).toEqual(['loud'])
      expect(last.working).toBe(2)
      expect(last.more).toBe(1)
    })

    it('fits a maximal card inside the size ActivityKit will accept', async () => {
      // ActivityKit caps a content state at 4KB encoded and APNs refuses a push
      // past it — which from every side looks like a relay that sent nothing.
      // The state grew from six fields to a row per agent, so this is MEASURED
      // rather than reasoned about, and it is the reason `STATE_BUDGET` exists
      // beside `ROWS_SHOWN` rather than the row count being trusted.
      //
      // Deliberately larger than anything a runner can produce: `detail` is cut
      // on the host at `farcooler_core::feed::SAID_WIDTH`, a hundred and twenty
      // characters, and the label and runner name are a preset and a hostname.
      // Multi-byte throughout, because the cap is bytes and these fields carry
      // whatever an agent said. Eight agents, which is twice the rows the card
      // draws, so the tail and the totals are being counted over more than fits.
      const calls = watchFetch()
      await ready()
      await running('term-1')
      for (let each = 0; each < 8; each++) {
        await post(
          '/v1/notify',
          {
            title: '✳'.repeat(300),
            subtitle: '✳'.repeat(600),
            terminal: crypto.randomUUID(),
            status: 'blocked',
            label: '✳'.repeat(300),
            startedAt: 1_755_000_000_000,
            insertions: 999999,
            deletions: 999999,
            commits: 255,
            trace: 'A'.repeat(88),
          },
          'mine',
        )
      }

      const activities = pushes(calls).filter(call => call.body.aps?.event)
      const last = activities[activities.length - 1]
      const encoded = new TextEncoder().encode(
        JSON.stringify(last.body.aps['content-state']),
      ).length
      expect(encoded).toBeLessThan(4096)
      // And the whole payload, which is the cap APNs itself applies. The state
      // is the largest part of it and not all of it.
      expect(new TextEncoder().encode(JSON.stringify(last.body)).length).toBeLessThan(4096)

      // **What this test actually measures, written down.** Every row here is
      // near three kilobytes on its own, so the first one crosses the budget
      // and the card goes out with NO lines at all — which is the right answer
      // and is the degenerate end of the cut. It means the row arithmetic is
      // not exercised anywhere above: this is a card with an enormous headline
      // and an empty roster, and it would still fit if the per-row measurement
      // were wrong in every direction. Pinned so that a change which starts
      // letting rows through is visible here rather than silently turning this
      // into a different test.
      //
      // The case where rows SURVIVE the cut is the one below, and the START —
      // the larger payload, and the push whose refusal means no card ever
      // appears — is the two after it. This assertion is an update.
      expect(last.body.aps.event).toBe('update')
      expect(last.body.aps['content-state'].rows).toEqual([])
      expect(last.body.aps['content-state'].more).toBe(8)
    })

    /// A fleet of `count` agents, and then the card that names them all.
    ///
    /// The order is the only way this route produces a maximal START, and it is
    /// not contrived: a card starts on the FIRST block it sees, so a fleet built
    /// on a phone that already offered a push-to-start token raises a card about
    /// one agent and never starts another. A phone that has not offered one
    /// accumulates the roster in silence — `startCard` returns before pushing
    /// when nothing on the account can raise a card — and the registration that
    /// follows is the app coming to the foreground, which is exactly the moment
    /// this happens in the field.
    async function startWithFleet(
      calls: Call[],
      count: number,
      notice: (terminal: string) => object,
    ) {
      await register('user_1')
      await pair('user_1', 'mine')
      const terminals = Array.from({ length: count }, () => crypto.randomUUID())
      for (const terminal of terminals) await post('/v1/notify', notice(terminal), 'mine')
      await register('user_1', { liveActivityStartToken: 'start-token' })
      // The same agent again rather than a new one, so the fleet is exactly
      // `count` and `+N more` can be checked against it.
      await post('/v1/notify', notice(terminals[0]), 'mine')
      return pushes(calls).filter(call => call.body.aps?.event === 'start')
    }

    /// The widest row a COMPLIANT runner can send.
    ///
    /// `detail` is cut on the host at `farcooler_core::feed::SAID_WIDTH` — a
    /// hundred and twenty CHARACTERS — and an agent's own words are routinely
    /// three bytes a character, which is where the wire parts company with the
    /// arithmetic behind `ROWS_SHOWN`: its 341-typical and 399-worst are ASCII
    /// rows. Nothing here exceeds a bound the runner enforces; it just spends
    /// each one in the units the cap is actually in.
    const SAID = '請'.repeat(120)
    const maximal = (terminal: string) => ({
      title: 'claude needs you',
      subtitle: SAID,
      terminal,
      status: 'blocked',
      label: 'claude-code-opus-5-x000',
      startedAt: 1_755_000_000_000,
      insertions: 999999,
      deletions: 999999,
      commits: 255,
      trace: 'A'.repeat(88),
    })

    it('starts a card for a maximal fleet inside the payload APNs will accept', async () => {
      // **The start is the push that has to fit, and nothing measured one.**
      // It carries everything an update does plus `attributes-type`,
      // `attributes` and `stale-date`, and APNs refuses rather than truncates —
      // so a start over the cap is not a card missing a row, it is no card at
      // all, for the whole run, reported as a 200.
      const calls = watchFetch()
      const starts = await startWithFleet(calls, 8, maximal)
      expect(starts.length).toBe(1)
      const state = starts[0].body.aps['content-state']

      // The cap APNs applies, on the whole payload.
      expect(bytes(starts[0].body)).toBeLessThan(4096)
      // And ActivityKit's separate one, on the state alone.
      expect(bytes(state)).toBeLessThan(4096)

      // **It fits WITH lines on it**, which is the half the maximal-card test
      // above cannot check: there the rows are so far past anything a runner
      // can send that every one of them is dropped, and a card with no roster
      // fits whatever the row arithmetic says. Here the rows are real, so the
      // cut is being asked the question it exists for.
      expect(state.rows.length).toBeGreaterThan(0)
      expect(state.rows.length).toBeLessThanOrEqual(4)
      // Nobody is lost. A row dropped for bytes is still an agent this card is
      // not naming, and the header has to say so.
      expect(state.more).toBe(8 - state.rows.length)
      // The cut drops whole rows and never truncates one. Half a question on a
      // lock screen is worse than a row that was not drawn.
      for (const row of state.rows) expect(row.detail).toBe(SAID)
    })

    it('drops rows rather than starting a card APNs would refuse', async () => {
      // The same deliberately-impossible fields as the maximal-card test above,
      // on the start path. Every bound in the row arithmetic belongs to a
      // runner that ships separately from this worker, so this is the case
      // `STATE_BUDGET` exists for: a build that widened one of them must cost
      // the card its rows and never cost the person the card.
      const calls = watchFetch()
      const starts = await startWithFleet(calls, 8, (terminal: string) => ({
        title: '✳'.repeat(300),
        subtitle: '✳'.repeat(600),
        terminal,
        status: 'blocked',
        label: '✳'.repeat(300),
        startedAt: 1_755_000_000_000,
        insertions: 999999,
        deletions: 999999,
        commits: 255,
        trace: 'A'.repeat(88),
      }))
      expect(starts.length).toBe(1)
      expect(bytes(starts[0].body)).toBeLessThan(4096)
      expect(bytes(starts[0].body.aps['content-state'])).toBeLessThan(4096)
      expect(starts[0].body.aps['content-state'].more).toBe(8)
    })

    it('leaves room in the payload for the alert and the envelope', async () => {
      // `STATE_BUDGET` bounds the state and nothing bounded the sum. APNs caps
      // the WHOLE payload, so a budget widened to fill the cap on its own puts
      // every alerting push over it — and a refused push is indistinguishable,
      // from every side, from a relay that sent nothing.
      //
      // The envelope is MEASURED off a real start rather than reasoned about,
      // and off a start because that is the largest one: `attributes-type`,
      // `attributes` and `stale-date` ride only on it.
      const calls = watchFetch()
      const starts = await startWithFleet(calls, 8, maximal)
      expect(starts.length).toBe(1)
      const start = starts[0]
      const envelope = bytes(start.body) - bytes(start.body.aps['content-state']) -
        bytes(start.body.aps.alert)
      expect(envelope).toBeGreaterThan(0)

      // The worst alert either kind of push can carry. A start's title is the
      // fleet header, which is short; an update's is cut to
      // `ALERT_TITLE_BUDGET`, which is not — so the update's is the one the
      // budget has to survive, and both bodies are cut to `ALERT_BODY_BUDGET`.
      const worstAlert = ALERT_TITLE_BUDGET + ALERT_BODY_BUDGET + bytes({ title: '', body: '' })
      expect(STATE_BUDGET + worstAlert + envelope).toBeLessThanOrEqual(4096)
    })
  })
})

// MARK: - Cards nothing is left to end

/// The rule this whole route exists for: a card outlives whatever raised it.
///
/// It is held by the relay and ended by a `done` that only the runner can send,
/// so anything that stops the runner from sending one strands the card — and
/// what it strands is the worst possible sentence to strand, because a blocked
/// card with no question of its own reads "Waiting for your answer" when nothing
/// is waiting. Two ways it happened, both reported live: the terminal went away
/// while its card was up, and the daemon restarted with its memory of what each
/// terminal was doing rebuilt empty. The second means every runner update
/// orphaned every card that was up.
// MARK: - Fitting the cap APNs enforces

/// What `cut` is actually for, as opposed to how long its answer is.
///
/// Nothing measured anything but the length. `cut` could return its input
/// REVERSED and every test in this file went on passing, because the three
/// callers all feed a payload whose size is what gets asserted — so the one
/// property the function exists for, that what comes back is the beginning of
/// what went in and is still decodable UTF-8, was guarded by nothing.
describe('cutting a line to a byte budget', () => {
  /// Three bytes each in UTF-8, and one UTF-16 unit each. A budget that is not a
  /// multiple of three therefore cannot be spent exactly, which is the case a
  /// naive `slice` gets wrong.
  const wide = 'ながいながいながいながい'
  /// Four bytes, and a SURROGATE PAIR: two UTF-16 units, so `slice` can halve it
  /// and produce a lone surrogate — which `JSON.stringify` writes as an escape
  /// the app decodes to a replacement character.
  const emoji = '🐟'

  function size(text: string): number {
    return new TextEncoder().encode(text).length
  }

  it('gives back exactly what it was given when that already fits', () => {
    expect(cut('short', 128)).toBe('short')
    expect(cut('', 0)).toBe('')
  })

  it('gives back the whole line at exactly the budget, and not a byte less', () => {
    // The off-by-one on the cheap side. `<=` rather than `<` on the early
    // return, and `>` rather than `>=` in the loop: a line that fits perfectly
    // is a line that fits.
    expect(size(wide)).toBe(36)
    expect(cut(wide, 36)).toBe(wide)
    expect(cut('abc', 3)).toBe('abc')
  })

  it('gives back a PREFIX of its input, never anything else', () => {
    // The property a reversed return violates, and the one every caller assumes:
    // a lock screen shows the beginning of what the agent said.
    const cropped = cut(wide, 20)
    expect(wide.startsWith(cropped)).toBe(true)
    expect(cropped.length).toBeLessThan(wide.length)
    expect(cut('abcdefgh', 3)).toBe('abc')
  })

  it('never splits a multi-byte character', () => {
    // Twenty bytes of a three-byte alphabet is six characters and two bytes
    // left over, and the two bytes are not spent: half a UTF-8 sequence is not
    // a shorter string, it is a string the app cannot decode.
    const cropped = cut(wide, 20)
    expect(cropped).toBe('ながいながい')
    expect(size(cropped)).toBe(18)
    expect(size(cropped)).toBeLessThanOrEqual(20)
  })

  it('never splits a surrogate pair', () => {
    // `for...of` iterates code points, so the pair is taken or left whole. A
    // `slice` at the same budget would leave a lone surrogate here.
    const school = emoji.repeat(4)
    expect(size(school)).toBe(16)
    const cropped = cut(school, 10)
    expect(cropped).toBe(emoji.repeat(2))
    expect([...cropped].every(character => size(character) === 4)).toBe(true)
    // Said again as the property, because this is what a lone surrogate breaks:
    // the payload has to survive the round trip through JSON.
    expect(JSON.parse(JSON.stringify(cropped))).toBe(cropped)
    expect(cropped).not.toContain('�')
  })

  it('drops a whole character rather than overspend by one byte', () => {
    // One byte under the character's width is the same answer as one byte under
    // the whole character: the budget is a ceiling and there is no such thing as
    // paying two thirds of a code point.
    for (const budget of [17, 18, 19, 20]) {
      const cropped = cut(wide, budget)
      expect(size(cropped)).toBeLessThanOrEqual(budget)
      expect(wide.startsWith(cropped)).toBe(true)
      expect(size(cropped) % 3).toBe(0)
    }
    expect(cut(wide, 2)).toBe('')
  })

  it('counts bytes and not characters', () => {
    // The whole reason it exists. Twelve characters of an agent's own words can
    // be thirty-six bytes, and the cap APNs applies is on the bytes.
    expect(cut(wide, 12)).toBe('ながいな')
    expect(cut('abcdefghijkl', 12)).toBe('abcdefghijkl')
  })

  it('is what the alert budgets are spent through', () => {
    // The link between the arithmetic above and the two constants the payload
    // tests add up. Asserted against the exported budgets rather than against
    // 128 and 512, so raising one moves this with it.
    const title = 'ながい'.repeat(200)
    expect(size(cut(title, ALERT_TITLE_BUDGET))).toBeLessThanOrEqual(ALERT_TITLE_BUDGET)
    expect(size(cut(title, ALERT_BODY_BUDGET))).toBeLessThanOrEqual(ALERT_BODY_BUDGET)
    expect(title.startsWith(cut(title, ALERT_TITLE_BUDGET))).toBe(true)
  })
})

describe('/v1/notify/retire', () => {
  async function ready() {
    await register('user_1', { liveActivityStartToken: 'start-token' })
    await pair('user_1', 'mine')
  }

  /// A card that is up, addressable, and known to be leading with `terminal`.
  ///
  /// Both halves are needed and only the relay can supply the first: the app
  /// files an update token and knows nothing about which agent the card is
  /// about, so a card whose leader has never been pushed is one no terminal in a
  /// sweep can be shown to be about. That is written straight into the row here
  /// rather than through a `working` notify, so the pushes a test counts are
  /// only the ones the retirement itself produced.
  async function running(terminal: string, token = 'update-token') {
    await env.DB.prepare(
      `INSERT INTO install_cards
         (id, account_id, update_token, leader_terminal, leader_status, updated_at)
       VALUES (?, ?, ?, ?, 'blocked', ?)
       ON CONFLICT (account_id) DO UPDATE SET update_token = excluded.update_token,
                                              leader_terminal = excluded.leader_terminal,
                                              updated_at = excluded.updated_at`,
    )
      .bind(crypto.randomUUID(), 'user_1', token, terminal, Date.now())
      .run()
  }

  it('ends the card and forgets the row', async () => {
    const calls = watchFetch()
    await ready()
    await running('term-1')

    const response = await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ retired: 1 })

    const [activity] = pushes(calls)
    expect(activity.url).toContain('/device/update-token')
    expect(activity.body.aps.event).toBe('end')
    // At once, not after the minute a FINISHED card gets. There is no last word
    // to leave up — the runner has just said it cannot account for the run — so
    // every second the card stays is a second of the lock screen stating
    // something that stopped being true.
    expect(activity.body.aps['dismissal-date']).toBe(activity.body.aps.timestamp)
    // Nothing left to name. The card is gone before anything on it could be
    // read, and a leader here would be a sentence about a run the runner has
    // just said it cannot account for.
    expect(activity.body.aps['content-state']).toEqual({
      terminal: '',
      label: '',
      machine: '',
      status: 'done',
      detail: '',
    })

    const rows = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(rows.results?.length).toBe(0)
  })

  it('wakes nobody', async () => {
    // The reason this is not a `done`. A card coming down is not news: the
    // person closed the pane themselves, or updated their runner. A buzz per
    // orphaned card would be this feature interrupting somebody to announce its
    // own housekeeping — and on a restart it would do it once per terminal.
    const calls = watchFetch()
    await ready()
    await running('term-1')
    await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')

    expect(pushes(calls).length).toBe(1)
    expect(pushes(calls)[0].headers['apns-push-type']).toBe('liveactivity')
    expect(pushes(calls)[0].body.aps.alert).toBeUndefined()
  })

  it('forgets a card it cannot address, and lets the next run have one', async () => {
    // The same bounded hole `done` has always had, and no worse: an update
    // token exists only once the app has run and reported it, so a card the
    // relay started blind has no address and never will until somebody opens
    // the app. Nothing is pushed at the sentinel — see `TOKEN_UNKNOWN` — the
    // row goes anyway, and the abandoned card clears itself on the `stale-date`
    // its start carried.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')

    expect(pushes(calls).filter(call => call.body.aps?.event).map(call => call.body.aps.event))
      .toEqual(['start'])
    const rows = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(rows.results?.length).toBe(0)

    // Which is the point of deleting it: the row is what a second start would
    // collide with, so a terminal whose card was retired is not refused a card
    // for every run that follows.
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(2)
  })

  it('leaves the card alone while any agent it was not asked about is still going', async () => {
    // A runner names the terminals it cannot account for, and an agent that is
    // still working — or still blocked, which is the case that matters — is not
    // among them. Ending the card would clear the lock screen of a running agent
    // because a different one stopped.
    //
    // **What decides it is the fleet, not the headline.** It used to be whether
    // the sweep named the one agent the card was about; with a row each, a
    // retired terminal is a row leaving the roster and the card is still true
    // about everybody else. So this holds even when the sweep names the agent
    // that WAS headlining, which the old rule got exactly backwards.
    const calls = watchFetch()
    await ready()
    await running('term-2', 'second-token')
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')
    await post('/v1/notify', { title: 'codex', terminal: 'term-2', status: 'working' }, 'mine')
    const before = pushes(calls).length

    const response = await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')
    expect(await response.json()).toEqual({ retired: 0 })

    // Silent: no push at all, which is the whole reason this route is not folded
    // into `/v1/notify`. A card coming down, or not, is not news.
    expect(pushes(calls).length).toBe(before)
    const card = await env.DB.prepare(`SELECT id FROM install_cards`).first<any>()
    expect(card).toBeTruthy()
    // And the retired agent is out of the roster, so the header stops counting
    // it. A row nothing retired would read "1 needs you" over a runner that
    // restarted an hour ago.
    const left = await env.DB.prepare(`SELECT terminal FROM live_activities`).all<any>()
    expect(left.results).toEqual([{ terminal: 'term-2' }])
  })

  it('leaves the card alone while an agent it was not asked about is still BLOCKED', async () => {
    // The same rule as above and the case that actually matters, which every
    // fixture in this block missed: the survivor was always `working`, so the
    // `blocked` half of `left.some(...)` decided nothing and could be deleted
    // with the suite still green.
    //
    // A blocked agent is a person waiting for a question they have been asked.
    // Taking its card down because a DIFFERENT agent's terminal went away is the
    // single worst thing this route can do — it deletes the one notification the
    // whole product exists to deliver, and it does it silently.
    const calls = watchFetch()
    await ready()
    await running('term-2', 'second-token')
    await post('/v1/notify', { title: 'codex', terminal: 'term-1', status: 'working' }, 'mine')
    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-2', status: 'blocked' },
      'mine',
    )
    const before = pushes(calls).length

    const response = await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')
    expect(await response.json()).toEqual({ retired: 0 })

    expect(pushes(calls).length).toBe(before)
    expect(await cardOf('user_1')).toBeTruthy()
    expect(await roster('user_1')).toEqual(['term-2'])
  })

  it('takes the card down when the last agent left was merely to review', async () => {
    // The other side of the same clause, and the reason it names two tiers
    // rather than "anything at all". A `done` row stays on the roster and keeps
    // being counted as "to review" — that is what the header's middle number
    // means — but it cannot on its own keep a card on the lock screen. So a
    // sweep that leaves nothing but finished agents behind ends the card, and
    // this is the case that separates `blocked || working` from `left.length`.
    const calls = watchFetch()
    await ready()
    await running('term-2', 'second-token')
    await post('/v1/notify', { title: 'codex', terminal: 'term-1', status: 'working' }, 'mine')
    await post('/v1/notify', { title: 'claude finished', terminal: 'term-2', status: 'done' }, 'mine')

    const response = await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')
    expect(await response.json()).toEqual({ retired: 1 })

    const ends = pushes(calls).filter(call => call.body.aps?.event === 'end')
    expect(ends.length).toBe(1)
    expect(ends[0].url).toContain('second-token')
    expect(await cardOf('user_1')).toBe(null)
  })

  it('ends a card the relay knows nothing about, because nothing is left to be about', async () => {
    // The app filed an update token for a card this relay holds no roster for —
    // an `end` that raced the report, a card left over from an older build, or
    // simply nothing that has notified in a day.
    //
    // This used to be left alone, on the argument that a card with no known
    // leader could not be shown to be about any terminal in the sweep. That
    // argument does not survive rows: an empty roster is not "unknown", it is
    // "nothing on this account has said anything", and a working agent says
    // something every ten seconds. So a runner saying its runs are over, against
    // a relay that can name no live agent, is enough — and leaving the card up
    // is leaving one that reads about nobody.
    const calls = watchFetch()
    await ready()
    await post(
      '/v1/devices/activity',
      { updateToken: 'orphan-token' },
      await sessionFor('user_1'),
    )

    const response = await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')
    expect(await response.json()).toEqual({ retired: 1 })
    // Immediately, and with no alert: see `Dismissal`.
    const [ended] = pushes(calls)
    expect(ended.body.aps.event).toBe('end')
    expect(ended.body.aps.alert).toBeUndefined()
    const rows = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(rows.results?.length).toBe(0)
  })

  it('will not end an activity belonging to another account', async () => {
    // The same rule as the alert: a machine says which terminals, never whose.
    // Two runners cannot mint the same UUID, but the account clause is what
    // makes that a fact about the query rather than a fact about UUIDs.
    //
    // **The other account needs a ROSTER ROW, not only a card.** This test had
    // one card and no rows, so the `DELETE FROM live_activities` this route
    // runs first never had another account's row in front of it: dropping
    // `account_id = ?` from that statement deleted nothing extra and the test
    // stayed green. A sweep is a list of terminal UUIDs from a runner that
    // cannot see this table, and the clause is the entire reason it cannot
    // reach across an account — so the row it must not reach has to be there.
    const calls = watchFetch()
    await ready()
    await env.DB.prepare(
      `INSERT INTO accounts (id, created_at) VALUES (?, ?) ON CONFLICT (id) DO NOTHING`,
    )
      .bind('user_2', Date.now())
      .run()
    await env.DB.prepare(
      `INSERT INTO install_cards
         (id, account_id, update_token, leader_terminal, leader_status, updated_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    )
      .bind(crypto.randomUUID(), 'user_2', 'their-update-token', 'term-1', 'blocked', Date.now())
      .run()
    // Deliberately the SAME terminal name the sweep below asks about, because
    // that is the only case the account clause decides: a name this account can
    // legitimately ask to retire, on a row it may not touch.
    await foreignAgent('user_2', 'term-1', { status: 'working' })

    const response = await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')
    expect(await response.json()).toEqual({ retired: 0 })
    expect(calls.every(call => !call.url.includes('their-update-token'))).toBe(true)
    const rows = await env.DB.prepare(`SELECT id FROM install_cards`).all<any>()
    expect(rows.results?.length).toBe(1)
    expect(await roster('user_2')).toEqual(['term-1'])
  })

  it("takes its own card down while another account's fleet is still working", async () => {
    // The test above stops one statement short and always did: the account it
    // sweeps for holds no card, so the route returns at `if (!running)` and
    // never reaches the question this one is about.
    //
    // That question is "is anybody still going", and it is asked of a FLEET.
    // `readFleet` is handed an account id, and nothing anywhere noticed if it
    // stopped being handed one: another person's busy agent would answer for
    // this person's card and hold it on the lock screen for as long as that
    // stranger kept working. Every other account clause in this route was
    // reachable; this one needed a card on THIS side of it to get to.
    const calls = watchFetch()
    await ready()
    await running('term-1')
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')

    // Somebody else, mid-run, on a terminal that happens to carry the same name.
    const theirs = await foreignCard('user_2')
    await foreignAgent('user_2', 'term-1', { status: 'working' })

    const response = await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')
    expect(await response.json()).toEqual({ retired: 1 })

    // Its own card, ended at its own address, and nothing sent to theirs.
    const ends = pushes(calls).filter(call => call.body.aps?.event === 'end')
    expect(ends.length).toBe(1)
    expect(ends[0].url).toContain('update-token')
    expect(calls.every(call => !call.url.includes('their-update-token'))).toBe(true)

    // And the other account is exactly as it was, in every column any write in
    // this service touches.
    expect(await cardOf('user_1')).toBe(null)
    expect(await cardOf('user_2')).toEqual(theirs)
    expect(await roster('user_2')).toEqual(['term-1'])
  })

  it('says nothing happened for terminals that never had a card', async () => {
    // Which is what makes the sweep safe to write the way it is. A runner names
    // every terminal it cannot account for — most of which are idle shells that
    // never had a card — rather than guessing at a table it cannot see.
    const calls = watchFetch()
    await ready()
    const response = await post('/v1/notify/retire', { terminals: ['term-1', 'term-2'] }, 'mine')

    expect(await response.json()).toEqual({ retired: 0 })
    expect(pushes(calls).length).toBe(0)
  })

  it('refuses a machine token nobody issued', async () => {
    watchFetch()
    await ready()
    expect((await post('/v1/notify/retire', { terminals: ['term-1'] }, 'theirs')).status).toBe(401)
    expect((await post('/v1/notify/retire', { terminals: ['term-1'] })).status).toBe(401)
  })

  it('needs a list of terminals', async () => {
    // A 400 rather than the shrug an unknown `status` gets on `/v1/notify`:
    // there is no forward-compatibility story to protect here, because a
    // request that names no terminals is asking for nothing at all.
    watchFetch()
    await ready()
    expect((await post('/v1/notify/retire', {}, 'mine')).status).toBe(400)
    expect((await post('/v1/notify/retire', { terminals: 'term-1' }, 'mine')).status).toBe(400)
    const empty = await post('/v1/notify/retire', { terminals: [] }, 'mine')
    expect(empty.status).toBe(200)
    expect(await empty.json()).toEqual({ retired: 0 })
  })

  it('takes only as many terminals as one request may name', async () => {
    // `RETIRE_LIMIT`. The largest honest sweep is one id per pane on the
    // machine, and the bound is what stops this route becoming a way to make the
    // worker spend a minute reading one caller's body. A real runner never trips
    // it: it sends its sweep in requests of that size, so the terminal past the
    // bound here arrives in the next one rather than being forgotten.
    watchFetch()
    await ready()
    await running('term-late')
    await post('/v1/notify', { title: 'claude', terminal: 'term-late', status: 'working' }, 'mine')
    const flood = [...Array(200)].map((_, index) => `term-${index}`)
    const response = await post(
      '/v1/notify/retire',
      { terminals: [...flood, 'term-late'] },
      'mine',
    )

    // `term-late` is past the bound, so it was not read and not retired: its row
    // survives, the fleet is still alive, and the card stays.
    expect(await response.json()).toEqual({ retired: 0 })
    const left = await env.DB.prepare(`SELECT terminal FROM live_activities`).all<any>()
    expect(left.results).toEqual([{ terminal: 'term-late' }])
    const card = await env.DB.prepare(`SELECT leader_terminal FROM install_cards`).first<any>()
    expect(card?.leader_terminal).toBe('term-late')
  })

  it('deletes rows past D1s parameter limit in chunks rather than failing', async () => {
    // `RETIRE_LIMIT` is a hundred and D1 refuses a statement with more than a
    // hundred bound parameters, so the largest honest sweep is exactly the size
    // that would break one statement. This route used to read a single row and
    // match ids in memory, which is why the limit stopped applying; the DELETE
    // that keeps the roster honest brings it back.
    watchFetch()
    await ready()
    const terminals = [...Array(100)].map((_, index) => `sweep-${index}`)
    for (const terminal of terminals.slice(0, 3)) {
      await post('/v1/notify', { title: 'claude', terminal, status: 'working' }, 'mine')
    }

    const response = await post('/v1/notify/retire', { terminals }, 'mine')
    expect(response.status).toBe(200)
    const left = await env.DB.prepare(`SELECT terminal FROM live_activities`).all<any>()
    expect(left.results).toEqual([])
  })
})

// MARK: - The management screen, and what it must not carry

/// What `/v1/account` answers with, which nothing checked the SHAPE of.
///
/// The route's own comment says "Never the tokens — not the push tokens, not
/// the daemon token hashes. A screen that lists devices needs to name them, not
/// to be able to become them." Nothing enforced that sentence. Adding
/// `push_token` to the SELECT and the object built from it left the suite
/// green, and what that ships is every device's push token to anyone holding a
/// session — which is the ability to notify that person's phone with anything,
/// from anywhere, for as long as the token lives.
///
/// Two assertions, deliberately overlapping. The key sets catch a column that
/// was added and mapped; the sweep for the secrets themselves catches one that
/// was mapped under an innocent name.
describe('/v1/account', () => {
  it('names the devices and machines without handing back a way to become them', async () => {
    watchFetch()
    const session = await sessionFor('user_1')
    await register('user_1', {
      label: 'iPhone',
      version: '1.2.3',
      pushToken: 'a-push-token-nobody-else-may-have',
      liveActivityStartToken: 'a-start-token-nobody-else-may-have',
    })
    const paired = await (await post('/v1/daemons', { label: 'Studio' }, session)).json<any>()

    const body = await (await post('/v1/account', {}, session)).json<any>()

    expect(Object.keys(body).sort()).toEqual(['devices', 'email', 'machines'])
    expect(Object.keys(body.devices[0]).sort()).toEqual([
      'id',
      'label',
      'platform',
      'state',
      'updatedAt',
      'version',
    ])
    expect(Object.keys(body.machines[0]).sort()).toEqual([
      'createdAt',
      'expiresAt',
      'id',
      'label',
      'lastSeenAt',
      'version',
    ])

    // And the same thing again over the whole wire, because a token returned
    // under a field named something else is the same token.
    const wire = JSON.stringify(body)
    for (const secret of [
      'a-push-token-nobody-else-may-have',
      'a-start-token-nobody-else-may-have',
      paired.token,
      await sha256(paired.token),
    ]) {
      expect(wire, secret).not.toContain(secret)
    }
  })

  it('lists this account and never another', async () => {
    // Three queries, three account clauses, and one screen. Every other test
    // that reads this route has a single account in the database, and against
    // one account a scoped read and an unscoped one return the same two lists
    // and the same email.
    //
    // **The other account is registered FIRST, and that ordering is the whole
    // test for the email.** The email lookup is a `.first()`, so an unscoped
    // version returns whichever account row SQLite reaches first — which, with
    // this account created first, is this account's own email. Written the
    // obvious way round, dropping that clause changed nothing anyone could see.
    watchFetch()
    await register('user_2', { label: 'Their iPhone', pushToken: 'their-device-token' })
    await post('/v1/daemons', { label: 'Their Studio' }, await sessionFor('user_2'))
    await register('user_1', { label: 'My iPhone' })
    await post('/v1/daemons', { label: 'My Studio' }, await sessionFor('user_1'))

    const body = await (await post('/v1/account', {}, await sessionFor('user_1'))).json<any>()

    expect(body.email).toBe('user_1@example.test')
    expect(body.devices.map((each: any) => each.label)).toEqual(['My iPhone'])
    expect(body.machines.map((each: any) => each.label)).toEqual(['My Studio'])
  })
})

// MARK: - Taking a device or a machine away

/// `/v1/daemons/revoke` had no test of any kind.
///
/// Not the account scoping, not the 400, not even the 401 — the list of
/// signed-in routes did not name it, so nothing in this file ever sent it a
/// request. It is the route that ends a machine's ability to notify, which is
/// the only thing a stolen daemon token can do at all, so it is the entire
/// remedy for a lost runner.
///
/// `revokeOwned` serves both tables from one function, so both routes are here:
/// one copy of the account clause, two routes resting on it, and a clause that
/// is checked IN the delete rather than before it — a separate ownership query
/// would leave a window between the check and the write, and there is no reason
/// to have the window.
describe('/v1/devices/revoke and /v1/daemons/revoke', () => {
  /// What one account still holds in a table, read straight from D1.
  ///
  /// Not through `/v1/account`: that route has its own scoping, and a test that
  /// asked it what survived would report a delete as scoped whenever the
  /// listing was scoped, which is the wrong question answered convincingly.
  async function idsIn(table: 'devices' | 'daemons', account: string): Promise<string[]> {
    const rows = await env.DB.prepare(`SELECT id FROM ${table} WHERE account_id = ? ORDER BY id`)
      .bind(account)
      .all<{ id: string }>()
    return (rows.results ?? []).map(row => row.id)
  }

  it('revokes a machine by the id the account listing gave for it', async () => {
    // Both halves of the only flow there is. Pairing returns a TOKEN and never
    // an id, so the one id the app can revoke by is the one `/v1/account`
    // handed it, and a test that invented an id would not be exercising the
    // pair of routes anybody uses.
    watchFetch()
    const session = await sessionFor('user_1')
    await post('/v1/daemons', { label: 'Studio' }, session)
    const listed = await (await post('/v1/account', {}, session)).json<any>()
    expect(listed.machines.map((each: any) => each.label)).toEqual(['Studio'])

    const response = await post('/v1/daemons/revoke', { id: listed.machines[0].id }, session)

    expect(await response.json()).toEqual({ ok: true })
    expect(await idsIn('daemons', 'user_1')).toEqual([])
  })

  it('stops a revoked machine notifying, which is the point of the route', async () => {
    // What revoking is FOR. A runner someone no longer controls holds a bearer
    // token that is good for a year, and this is the only thing that ends it.
    watchFetch()
    const session = await sessionFor('user_1')
    await register('user_1')
    const paired = await (await post('/v1/daemons', { label: 'Studio' }, session)).json<any>()
    const [id] = await idsIn('daemons', 'user_1')
    expect((await post('/v1/notify', { title: 'hi' }, paired.token)).status).toBe(200)

    await post('/v1/daemons/revoke', { id }, session)

    expect((await post('/v1/notify', { title: 'hi' }, paired.token)).status).toBe(401)
  })

  it('will not revoke a machine belonging to another account', async () => {
    // A daemon id is a UUID, so this is not a guess anyone makes twice — but it
    // is a value the other account has SEEN, on its own screen, and the clause
    // is what makes ownership a fact about the statement rather than a fact
    // about how hard the id is to come by.
    watchFetch()
    await post('/v1/daemons', { label: 'Mine' }, await sessionFor('user_1'))
    await post('/v1/daemons', { label: 'Theirs' }, await sessionFor('user_2'))
    const [theirs] = await idsIn('daemons', 'user_2')

    const response = await post('/v1/daemons/revoke', { id: theirs }, await sessionFor('user_1'))

    expect(await response.json()).toEqual({ ok: false })
    expect(await idsIn('daemons', 'user_2')).toEqual([theirs])
    // And nothing of this account's went in its place.
    expect((await idsIn('daemons', 'user_1')).length).toBe(1)
  })

  it('revokes a device this account registered', async () => {
    watchFetch()
    const session = await sessionFor('user_1')
    await register('user_1')
    const [mine] = await idsIn('devices', 'user_1')

    expect(await (await post('/v1/devices/revoke', { id: mine }, session)).json()).toEqual({
      ok: true,
    })
    expect(await idsIn('devices', 'user_1')).toEqual([])
  })

  it('will not revoke a device belonging to another account', async () => {
    // The one with a phone on the end of it: a device row is where a push token
    // lives, so deleting somebody else's is silencing their notifications
    // outright, and nothing in the app would explain why they stopped.
    watchFetch()
    await register('user_1')
    await register('user_2', { pushToken: 'their-device-token' })
    const [theirs] = await idsIn('devices', 'user_2')

    const response = await post('/v1/devices/revoke', { id: theirs }, await sessionFor('user_1'))

    expect(await response.json()).toEqual({ ok: false })
    expect(await idsIn('devices', 'user_2')).toEqual([theirs])
    expect((await idsIn('devices', 'user_1')).length).toBe(1)
  })

  it('says nothing happened for an id nobody holds', async () => {
    // `ok` reports whether a row actually went. It used to be `true`
    // unconditionally, which made revoking nothing indistinguishable from a
    // real delete — and the app removes the row optimistically on that answer,
    // so a no-op read as success right up until the list reloaded.
    watchFetch()
    const session = await sessionFor('user_1')
    await register('user_1')
    await post('/v1/daemons', {}, session)

    const missing = crypto.randomUUID()
    expect(await (await post('/v1/devices/revoke', { id: missing }, session)).json()).toEqual({
      ok: false,
    })
    expect(await (await post('/v1/daemons/revoke', { id: missing }, session)).json()).toEqual({
      ok: false,
    })
    // And the rows that were there are still there.
    expect((await idsIn('devices', 'user_1')).length).toBe(1)
    expect((await idsIn('daemons', 'user_1')).length).toBe(1)
  })

  it('needs an id, and a string one', async () => {
    // Typed rather than merely present: a non-string id reaches the D1 binder
    // and throws, which the top-level catch turns into a 500 for what is a bad
    // request.
    watchFetch()
    const session = await sessionFor('user_1')
    for (const path of ['/v1/devices/revoke', '/v1/daemons/revoke']) {
      expect((await post(path, {}, session)).status, path).toBe(400)
      expect((await post(path, { id: '' }, session)).status, path).toBe(400)
      expect((await post(path, { id: 42 }, session)).status, path).toBe(400)
    }
  })
})

// MARK: - One install's card is never another install's

/// The write half of the card's account scoping, which was held by nothing.
///
/// `a00fe5b` did this for `live_activities`, and the shape of the gap is the
/// same: with one account's rows in the table, a scoped write and an unscoped
/// one are indistinguishable. It is worse here, because `install_cards` is
/// `UNIQUE (account_id)` — the table CANNOT hold two accounts' cards unless a
/// fixture puts a second one there, so no test in this file had ever seen a
/// second row, and all six `WHERE account_id = ?` clauses could be deleted
/// together with the suite still green. The read side was already covered and
/// already goes red without its clause; only the writes were open.
///
/// One test per write, and each asserts the same thing about the other
/// account's card: not merely that it still exists, but that every column a
/// write in this service touches is exactly as it was left.
describe("one install's card is never another install's", () => {
  /// A registered phone, a paired machine called Studio, and a session.
  async function ready() {
    await register('user_1', { liveActivityStartToken: 'start-token' })
    await pair('user_1', 'mine')
    return await sessionFor('user_1')
  }

  it('files an update token without touching anybody else', async () => {
    // `/v1/devices/activity`, the dismissal arm. The person swiped THEIR card
    // away, and unscoped this sets every account's card to the sentinel with a
    // dismissal stamp on it — which `pushActivity` then reads as "no card, and
    // they meant it", so every other install goes silent until something blocks.
    watchFetch()
    const session = await ready()
    await post('/v1/devices/activity', { updateToken: 'mine-token' }, session)
    const theirs = await foreignCard('user_2')

    await post('/v1/devices/activity', { updateToken: null, dismissed: true }, session)

    const mine = await cardOf('user_1')
    expect(mine?.update_token).toBe('')
    expect(mine?.dismissed_at).toBeGreaterThan(0)
    expect(await cardOf('user_2')).toEqual(theirs)
  })

  it('forgets its own ended card and nobody else\'s', async () => {
    // `/v1/devices/activity`, the arm for a card that merely ended. Unscoped,
    // one app reporting that its activity is over deletes the row for every
    // card the relay is holding.
    watchFetch()
    const session = await ready()
    await post('/v1/devices/activity', { updateToken: 'mine-token' }, session)
    const theirs = await foreignCard('user_2')

    await post('/v1/devices/activity', { updateToken: null }, session)

    expect(await cardOf('user_1')).toBe(null)
    expect(await cardOf('user_2')).toEqual(theirs)
  })

  it('retires its own card and nobody else\'s', async () => {
    // `/v1/notify/retire`. There is a cross-account test for this route
    // already, and it stops one statement short: this account had no card of
    // its own there, so the route returned before it ever reached the delete.
    // A runner that restarts sweeps every terminal it cannot account for, so
    // this is the ordinary path rather than an exotic one.
    watchFetch()
    const session = await ready()
    await post('/v1/devices/activity', { updateToken: 'mine-token' }, session)
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
    const theirs = await foreignCard('user_2')

    const response = await post('/v1/notify/retire', { terminals: ['term-1'] }, 'mine')

    expect(await response.json()).toEqual({ retired: 1 })
    expect(await cardOf('user_1')).toBe(null)
    expect(await cardOf('user_2')).toEqual(theirs)
  })

  it('ends its own card when its last agent stops, and nobody else\'s', async () => {
    // `/v1/notify`, the arm for a fleet with nothing left to be about. Unscoped,
    // the last agent on ONE runner finishing takes down every card in the
    // service — including the ones with a blocked agent waiting on them, which
    // is the single push this product exists to deliver.
    watchFetch()
    const session = await ready()
    await post('/v1/devices/activity', { updateToken: 'mine-token' }, session)
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
    const theirs = await foreignCard('user_2')

    await post('/v1/notify', { title: 'claude is done', terminal: 'term-1', status: 'done' }, 'mine')

    expect(await cardOf('user_1')).toBe(null)
    expect(await cardOf('user_2')).toEqual(theirs)
  })

  it('forgets its own stale claim and nobody else\'s', async () => {
    // `/v1/notify`, the `CLAIM_MEMORY_MS` arm. A card the relay started blind
    // and never heard about again is dropped so a fresh one can be raised —
    // and unscoped, every other install's card is dropped with it, including
    // the addressable ones that were being moved in place quite happily.
    const calls = watchFetch()
    await ready()
    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )
    await env.DB.prepare(`UPDATE install_cards SET updated_at = ? WHERE account_id = ?`)
      .bind(Date.now() - 2 * 60 * 60 * 1000, 'user_1')
      .run()
    const theirs = await foreignCard('user_2')

    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    // The stale claim was dropped and a second card raised in its place, which
    // is what makes this the arm it is rather than the coalescing one.
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(2)
    expect(await cardOf('user_2')).toEqual(theirs)
  })

  it('moves its own leader and nobody else\'s', async () => {
    // `/v1/notify`, the update arm — the write that runs on every push of every
    // card in the service, so it is the one an unscoped clause reaches most
    // often. Unscoped, every account's card is recorded as leading with THIS
    // account's terminal, and the next push on each of them then decides what
    // it may show against a leader belonging to a stranger.
    watchFetch()
    const session = await ready()
    await post('/v1/devices/activity', { updateToken: 'mine-token' }, session)
    const theirs = await foreignCard('user_2')

    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    const mine = await cardOf('user_1')
    expect(mine?.leader_terminal).toBe('term-1')
    expect(mine?.leader_status).toBe('blocked')
    expect(await cardOf('user_2')).toEqual(theirs)
  })
})

// MARK: - Proving possession of a key

/// The two strings a registration carries to prove it holds Key A.
///
/// A real Ed25519 key, signed with `crypto.subtle`, because the thing under
/// test is a signature the relay refuses — and a stubbed verifier would agree
/// with whatever the route already believed.
async function deviceKey() {
  const pair = (await crypto.subtle.generateKey('Ed25519', true, [
    'sign',
    'verify',
  ])) as CryptoKeyPair
  const raw = new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey))
  return {
    /// The 32 bytes on their own, so a test can put this same key inside a blob
    /// of its own making and still sign with it.
    raw,
    keyA: `ssh-ed25519 ${base64(sshBlob(raw))} test@example`,
    async sign(message: string): Promise<string> {
      const signature = await crypto.subtle.sign(
        'Ed25519',
        pair.privateKey,
        new TextEncoder().encode(message),
      )
      return base64(new Uint8Array(signature))
    },
  }
}

/// The SSH wire encoding of a public key: two length-prefixed strings, the
/// algorithm name and the key bytes. This is what the fingerprint is over,
/// which is why the test builds it rather than hashing the key alone.
///
/// The algorithm and the body are both arguments, because the checks in
/// `parseEd25519` are about the BYTES: a blob that names another algorithm
/// inside itself, or carries the wrong number of bytes, cannot be written any
/// other way.
function sshBlob(raw: Uint8Array, algorithm = 'ssh-ed25519'): Uint8Array {
  const name = new TextEncoder().encode(algorithm)
  const out = new Uint8Array(4 + name.length + 4 + raw.length)
  new DataView(out.buffer).setUint32(0, name.length)
  out.set(name, 4)
  new DataView(out.buffer).setUint32(4 + name.length, raw.length)
  out.set(raw, 8 + name.length)
  return out
}

/// The 32 bytes of a fresh ed25519 public key, with no wire encoding around
/// them, so a test can put them inside a blob of its own choosing.
async function ed25519Raw(): Promise<Uint8Array> {
  const pair = (await crypto.subtle.generateKey('Ed25519', true, [
    'sign',
    'verify',
  ])) as CryptoKeyPair
  return new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey))
}

function base64(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
}

/// The three fields an updated app sends to prove it holds a key: a fresh one
/// every call, so no two devices in a test share a fingerprint by accident.
async function proof(deviceId: string) {
  const key = await deviceKey()
  return { deviceId, keyA: key.keyA, signature: await key.sign(deviceId) }
}

/// Register the way an updated app does: a key, and a signature over the device
/// id it is registering under.
async function registerProven(account: string, fields: Record<string, unknown> = {}) {
  const deviceId = crypto.randomUUID()
  const response = await register(account, { ...(await proof(deviceId)), ...fields })
  return { response, deviceId }
}

/// What the relay decided to store for a device, read back from D1 rather than
/// from the route's own answer.
async function deviceRow(pushToken = 'device-token') {
  return await env.DB.prepare(
    `SELECT id, account_id, label, key_a_fingerprint, state FROM devices WHERE push_token = ?`,
  )
    .bind(pushToken)
    .first<{
      id: string
      account_id: string
      label: string
      key_a_fingerprint: string | null
      state: string
    }>()
}

/// Promote a device the way a trusted one does once it has enrolled it.
async function promote(account: string, fingerprint: string) {
  return await post('/v1/devices/verify', { fingerprint }, await sessionFor(account))
}

describe('proof of possession at registration', () => {
  it('records the fingerprint of the key that signed, and nothing about the key', async () => {
    watchFetch()
    const { response } = await registerProven('user_1')
    expect(response.status).toBe(200)

    const row = await deviceRow()
    expect(row?.key_a_fingerprint).toMatch(/^SHA256:[A-Za-z0-9+/]{43}$/)
    // A row created by the new device is pending: it proves possession of a
    // key, which is not the same as a ceremony having enrolled it.
    expect(row?.state).toBe('pending')

    // The relay stores a fingerprint, never a key. If the key itself ever
    // appeared in a column, "never install a key the relay handed you" would go
    // back to being a rule someone has to remember.
    const everything = JSON.stringify(await env.DB.prepare(`SELECT * FROM devices`).first())
    expect(everything).not.toContain('AAAAC3Nza')
  })

  it('fingerprints a key the way ssh-keygen does', async () => {
    // A golden, from a real `ssh-keygen -lf`. This string is compared by eye
    // against what the other device shows and by string against what the Rust
    // client computes, so a fingerprint the relay invented for itself would
    // agree with nothing outside this file — and the one place it is used is a
    // comparison.
    const line =
      'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOoMTzNWaYOZBhg2HY8PVwkmuwqMQOqfrM9xATpdIMEm test@example'
    expect(await fingerprintOf(parseEd25519(line)!)).toBe(
      'SHA256:yjZGaYPt6bVurNagcMgxNBH8z8ldaacgkwyQoKhR430',
    )
    // A line whose LABEL names another algorithm, refused at the text before
    // anything is decoded, so that the error is about the key and not about
    // base64. This says nothing about the structural check — the comment here
    // used to claim it did — and what does is the block below.
    expect(parseEd25519(line.replace('ssh-ed25519 ', 'ssh-rsa '))).toBe(null)
    expect(parseEd25519('ssh-ed25519 not-base64')).toBe(null)
    expect(parseEd25519('')).toBe(null)
  })

  it('goes by the algorithm inside the blob, not the label in front of it', async () => {
    // The check the two tests that claimed it never reached. Both sent
    // `ssh-rsa AAAA…`, which is refused by the TEXT branch four lines into
    // `parseEd25519` — so the structural comparison could be deleted outright
    // and the whole suite stayed green.
    //
    // What it is for is the other shape: a line labelled `ssh-ed25519` whose
    // bytes name something else. The label is text anyone can write and the
    // wire encoding is what a verifier goes by, so a relay that trusted the
    // label would fingerprint an RSA blob as an ed25519 key and hand its 32
    // leading bytes to `importKey`.
    const raw = await ed25519Raw()
    expect(parseEd25519(`ssh-ed25519 ${base64(sshBlob(raw))}`)).not.toBe(null)
    expect(parseEd25519(`ssh-ed25519 ${base64(sshBlob(raw, 'ssh-rsa'))}`)).toBe(null)
    // And with no label at all, which is the form this function also accepts —
    // so the refusal above is about the bytes and not about the two words
    // disagreeing.
    expect(parseEd25519(base64(sshBlob(raw)))).not.toBe(null)
    expect(parseEd25519(base64(sshBlob(raw, 'ssh-rsa')))).toBe(null)
    expect(parseEd25519(base64(sshBlob(raw, 'ssh-ed25519-v2')))).toBe(null)
  })

  it('refuses a blob carrying anything but exactly 32 bytes, and nothing after them', async () => {
    // An ed25519 public key is 32 bytes. A blob with fewer, more, or trailing
    // bytes after the key is one two readers could disagree about — and they
    // would disagree about its FINGERPRINT too, which is the string a person
    // compares by eye at the confirmation and the only thing standing between
    // them and enrolling somebody else's key.
    const raw = await ed25519Raw()
    expect(parseEd25519(base64(sshBlob(raw)))).not.toBe(null)
    expect(parseEd25519(base64(sshBlob(raw.subarray(0, 31))))).toBe(null)
    expect(parseEd25519(base64(sshBlob(new Uint8Array([...raw, 0]))))).toBe(null)

    // Trailing bytes: a well-formed key with junk appended after it. The length
    // prefixes still parse, so only `key.next !== blob.length` catches this.
    const padded = new Uint8Array([...sshBlob(raw), 7, 7, 7])
    expect(parseEd25519(base64(padded))).toBe(null)
  })

  it('refuses a registration whose key is ed25519 only in its label', async () => {
    // The same confusion arriving through the route, which is where it would
    // matter: the fingerprint this stores is what a ceremony compares, and a
    // blob nobody else parses the same way is a fingerprint nobody else
    // computes the same way.
    watchFetch()
    // The SAME key inside the mislabelled blob and behind the signature, so the
    // signature really does verify and the structural check is the only thing
    // left standing between this and a 200. Signing with a different key would
    // make the route 400 for a reason that has nothing to do with the blob.
    const key = await deviceKey()
    const response = await register('user_1', {
      deviceId: 'device-1',
      keyA: `ssh-ed25519 ${base64(sshBlob(key.raw, 'ssh-rsa'))} test@example`,
      signature: await key.sign('device-1'),
    })
    expect(response.status).toBe(400)
    expect(await response.json()).toEqual({ error: 'keyA' })
    expect(await deviceRow()).toBe(null)
  })

  it('refuses a registration whose own fingerprint disagrees with its key', async () => {
    // Both screens show this string at the confirmation. A client computing it
    // differently from the relay would put two strings in front of a person who
    // is being asked to check that they match, and that is worth failing at
    // registration rather than discovering half way through a ceremony.
    watchFetch()
    const key = await deviceKey()
    const response = await register('user_1', {
      deviceId: 'device-1',
      keyA: key.keyA,
      signature: await key.sign('device-1'),
      fingerprint: 'SHA256:not-what-that-key-hashes-to',
    })
    expect(response.status).toBe(400)
    expect(await response.json()).toEqual({ error: 'fingerprint' })
    expect(await deviceRow()).toBe(null)
  })

  it('refuses a registration that shows a key but no signature', async () => {
    watchFetch()
    const key = await deviceKey()
    const response = await register('user_1', { deviceId: 'device-1', keyA: key.keyA })
    expect(response.status).toBe(400)
    expect(await deviceRow()).toBe(null)
  })

  it('refuses a registration whose signature does not verify', async () => {
    // The point of the whole task. Without this a session-holder registers any
    // fingerprint they have seen anywhere — off a screen, out of a QR — and the
    // account gate checks membership of a registry rather than that the device
    // in front of you holds the key it is showing.
    watchFetch()
    const mine = await deviceKey()
    const theirs = await deviceKey()
    const response = await register('user_1', {
      deviceId: 'device-1',
      keyA: theirs.keyA,
      signature: await mine.sign('device-1'),
    })
    expect(response.status).toBe(400)
    expect(await deviceRow()).toBe(null)
  })

  it('refuses a signature over something other than the device id it sent', async () => {
    watchFetch()
    const key = await deviceKey()
    const response = await register('user_1', {
      deviceId: 'device-1',
      keyA: key.keyA,
      signature: await key.sign('device-2'),
    })
    expect(response.status).toBe(400)
  })

  it('refuses a key that is not an ed25519 one', async () => {
    watchFetch()
    const key = await deviceKey()
    const response = await register('user_1', {
      deviceId: 'device-1',
      keyA: key.keyA.replace('ssh-ed25519', 'ssh-rsa'),
      signature: await key.sign('device-1'),
    })
    expect(response.status).toBe(400)
  })

  it('still registers a build that has never heard of keys', async () => {
    // Every shipped app is one of these. Refusing them would take push down for
    // everyone already installed on the day this deploys, and the design's own
    // answer for a device that never re-registers is that it shows as
    // unverified — which requires it to still be able to register at all.
    watchFetch()
    expect((await register('user_1')).status).toBe(200)
    const row = await deviceRow()
    expect(row?.key_a_fingerprint).toBe(null)
    expect(row?.state).toBe('verified')
  })

  it('gives an existing row its fingerprint and state when it re-registers', async () => {
    // CRITICAL, and the reason this task has a regression test at all. The
    // upsert names its updated columns explicitly, so a migration that adds
    // columns does not add them here: without the fix, an updated app
    // re-registers with a 200 and stays legacy forever — fingerprint NULL,
    // invisible to every ceremony, with nothing on any screen saying why.
    //
    // The state stays `verified`. This row predates the ceremony and was created
    // by the flow that was the old trust model; demoting every already-installed
    // device to pending would leave a fleet where nothing is verified and
    // nothing can promote anything, because promotion needs a trusted device.
    watchFetch()
    await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES (?, ?)`)
      .bind('user_1', Date.now())
      .run()
    await env.DB.prepare(
      `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
    )
      .bind('legacy-row', 'user_1', 'apns', 'device-token', 'iPhone', Date.now())
      .run()

    const { response } = await registerProven('user_1', { label: 'iPhone' })
    expect(response.status).toBe(200)

    const row = await deviceRow()
    expect(row?.id).toBe('legacy-row')
    expect(row?.key_a_fingerprint).toMatch(/^SHA256:/)
    expect(row?.state).toBe('verified')
  })

  it('drops back to pending when a different key arrives on the same device', async () => {
    // Possession of the new key is proven; a ceremony for it is not. The row
    // keeping `verified` would mean a key nobody ever enrolled inheriting the
    // standing of the one it replaced.
    watchFetch()
    await registerProven('user_1')
    await promote('user_1', (await deviceRow())!.key_a_fingerprint!)
    expect((await deviceRow())?.state).toBe('verified')

    await registerProven('user_1')
    expect((await deviceRow())?.state).toBe('pending')
  })

  it('tells the devices screen which rows are unverified', async () => {
    // The design's answer for a device that never re-registers, and for one
    // half way through a ceremony, is that it appears as unverified rather than
    // failing silently later. The app cannot say that unless the relay says it.
    watchFetch()
    await registerProven('user_1')
    const body = await (await post('/v1/account', {}, await sessionFor('user_1'))).json<any>()
    expect(body.devices[0].state).toBe('pending')
  })

  it('lets the same key move to a device whose push token changed', async () => {
    // A reinstall, or Apple reissuing a token. One row per key per account is a
    // unique index, so without this the insert violates it and registration
    // fails with a 500 — every time, for good, on the one path that has to keep
    // working for push to work at all.
    watchFetch()
    await registerProven('user_1')
    const key = await deviceKey()
    const response = await register('user_1', {
      pushToken: 'a-fresh-token',
      deviceId: 'device-2',
      keyA: key.keyA,
      signature: await key.sign('device-2'),
    })
    expect(response.status).toBe(200)
    expect((await deviceRow('a-fresh-token'))?.key_a_fingerprint).toMatch(/^SHA256:/)
  })
})

// MARK: - The one question the relay answers about a key

describe('/v1/devices/lookup', () => {
  it('finds only this account and never says whose a key is otherwise', async () => {
    // Scoped in the query rather than compared afterwards. A lookup by key alone
    // would return some device and leave the caller checking account ids, which
    // breaks the moment two accounts register the same public key — and turns
    // the route into a key-enumeration oracle for everyone else's devices.
    watchFetch()
    const key = await deviceKey()
    for (const [account, token] of [
      ['user_1', 'phone-1'],
      ['user_2', 'phone-2'],
    ]) {
      await register(account, {
        pushToken: token,
        label: `${account}'s phone`,
        deviceId: account,
        keyA: key.keyA,
        signature: await key.sign(account),
      })
    }
    const fingerprint = (await deviceRow('phone-1'))!.key_a_fingerprint!
    await promote('user_1', fingerprint)
    await promote('user_2', fingerprint)

    const mine = await (await post('/v1/devices/lookup', { fingerprint }, await sessionFor('user_1'))).json<any>()
    expect(mine.found).toBe(true)
    expect(mine.label).toBe("user_1's phone")

    const theirs = await (await post('/v1/devices/lookup', { fingerprint }, await sessionFor('user_2'))).json<any>()
    expect(theirs.label).toBe("user_2's phone")

    // A miss is a miss. No account, no id, no label, nothing that would tell a
    // stranger the key exists somewhere else.
    const nobody = await (await post('/v1/devices/lookup', { fingerprint }, await sessionFor('user_3'))).json<any>()
    expect(nobody).toEqual({ found: false })
  })

  it('finds a device whose ceremony has not completed, and says so', async () => {
    // The contract that makes onboarding possible at all. The row is `pending`
    // until a ceremony completes, and the trusted device cannot complete one
    // until this lookup has answered — so a lookup that required `verified`
    // would be waiting on its own result and every onboarding would end at "that
    // device is signed into a different account".
    //
    // The gate is account membership, which a registration that proved
    // possession under this account's session has already satisfied. The state
    // rides along so the caller can say a ceremony has not finished without
    // asking a second time.
    watchFetch()
    await registerProven('user_1', { label: 'New iPhone' })
    const fingerprint = (await deviceRow())!.key_a_fingerprint!
    const session = await sessionFor('user_1')

    const pending = await (
      await post('/v1/devices/lookup', { fingerprint }, session)
    ).json<any>()
    expect(pending).toEqual({ found: true, label: 'New iPhone', state: 'pending' })

    await promote('user_1', fingerprint)
    const verified = await (
      await post('/v1/devices/lookup', { fingerprint }, session)
    ).json<any>()
    expect(verified).toEqual({ found: true, label: 'New iPhone', state: 'verified' })
  })

  it('does not find a row on another account whatever its state', async () => {
    // The account is the gate, and it is the only thing the state's arrival
    // must not soften. Pending or verified, someone else's device is not yours
    // — and a miss says nothing that would separate a key registered on another
    // account from a key registered nowhere.
    watchFetch()
    await register('user_2', { pushToken: 'their-phone', label: "Their phone", ...(await proof('their-device')) })
    const fingerprint = (await deviceRow('their-phone'))!.key_a_fingerprint!
    const session = await sessionFor('user_1')

    expect(await (await post('/v1/devices/lookup', { fingerprint }, session)).json()).toEqual({
      found: false,
    })

    await promote('user_2', fingerprint)
    const answer = await (await post('/v1/devices/lookup', { fingerprint }, session)).json<any>()
    expect(answer).toEqual({ found: false })
    // Said again as a property rather than a shape, because this is the one
    // that must survive every later change to the response.
    expect(JSON.stringify(answer)).not.toContain('user_2')
    expect(JSON.stringify(answer)).not.toContain('Their phone')
    expect(answer.state).toBeUndefined()
  })

  it('needs a fingerprint, and a session', async () => {
    watchFetch()
    expect((await post('/v1/devices/lookup', {})).status).toBe(401)
    expect((await post('/v1/devices/lookup', {}, await sessionFor('user_1'))).status).toBe(400)
  })
})

describe('/v1/devices/verify', () => {
  it('promotes only from pending, and only on this account', async () => {
    watchFetch()
    await registerProven('user_1')
    const fingerprint = (await deviceRow())!.key_a_fingerprint!

    // Another account cannot promote a row it does not own, and is not told
    // that there was one.
    expect(await (await promote('user_2', fingerprint)).json()).toEqual({ ok: false })
    expect((await deviceRow())?.state).toBe('pending')

    expect(await (await promote('user_1', fingerprint)).json()).toEqual({ ok: true })
    expect((await deviceRow())?.state).toBe('verified')

    // Already verified is not a promotion. Answering `ok` a second time would
    // make "a ceremony completed just now" indistinguishable from "this key was
    // already here", which is the one thing the caller is asking.
    expect(await (await promote('user_1', fingerprint)).json()).toEqual({ ok: false })
  })

  it('will not promote a pending row that has gone stale', async () => {
    // A pending row expires. It is created before a ceremony and promoted at the
    // end of one, so anything older than that is not a ceremony finishing — it
    // is a row nobody came back for, and a promotion for it would be a
    // registration from any earlier time being completed by a later session.
    watchFetch()
    await registerProven('user_1')
    const fingerprint = (await deviceRow())!.key_a_fingerprint!
    await env.DB.prepare(`UPDATE devices SET updated_at = ? WHERE key_a_fingerprint = ?`)
      .bind(Date.now() - 25 * 60 * 60 * 1000, fingerprint)
      .run()

    expect(await (await promote('user_1', fingerprint)).json()).toEqual({ ok: false })
    expect((await deviceRow())?.state).toBe('pending')
  })
})

// MARK: - Counting without knowing who

/// `analytics.ts` had no tests at all, and the property it was written for is
/// the one nothing could have noticed losing.
///
/// The whole argument for Analytics Engine over a table in D1 is that no account
/// id lands here, so a deletion request has nothing to sweep — and that only
/// holds because the hash is MONTHLY SALTED. Pin the month and the id becomes
/// permanent: the series turns into a per-user history by accumulation, which is
/// exactly the thing the design says it cannot become. `${salt}:${month}` could
/// be shortened to `${salt}` and nothing anywhere would have failed.
describe('the anonymous id', () => {
  /// Run something with the clock parked in a given month.
  ///
  /// `anonymousId` reads the month off `new Date()`, which is the only way to
  /// observe the rotation the design depends on. Real timers are restored on the
  /// way out however it ends, because every other test in this file reads the
  /// real clock.
  async function inMonth<T>(when: string, body: () => Promise<T>): Promise<T> {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(when))
    try {
      return await body()
    } finally {
      vi.useRealTimers()
    }
  }

  it('is the same all month, which is what DAU and MAU are counted from', async () => {
    // `count(distinct index1)` over a day or a month is the whole query. It
    // needs one account to produce ONE id for the length of the window and it
    // needs nothing else.
    const first = await inMonth('2026-03-01T00:00:00Z', () => anonymousId('user_1', 'salt'))
    const later = await inMonth('2026-03-28T23:59:59Z', () => anonymousId('user_1', 'salt'))
    expect(later).toBe(first)
  })

  it('is a different id next month, which is what makes it forgettable', async () => {
    // The property the whole storage decision rests on. Once the salt rotates,
    // last month's hashes cannot be matched to this month's — so the counters
    // survive a deletion request and the history cannot accumulate into a
    // per-user one. An id that outlived the month would be an account id in
    // everything but name.
    const march = await inMonth('2026-03-31T23:59:59Z', () => anonymousId('user_1', 'salt'))
    const april = await inMonth('2026-04-01T00:00:00Z', () => anonymousId('user_1', 'salt'))
    expect(april).not.toBe(march)
    // A YEAR is not a month. Rounding the key to `YYYY` would keep the id for
    // twelve months and still pass the test above.
    const nextJanuary = await inMonth('2027-01-01T00:00:00Z', () => anonymousId('user_1', 'salt'))
    expect(nextJanuary).not.toBe(march)
    expect(nextJanuary).not.toBe(april)
  })

  it('separates two accounts, and separates two salts', async () => {
    await inMonth('2026-03-15T00:00:00Z', async () => {
      const mine = await anonymousId('user_1', 'salt')
      expect(await anonymousId('user_2', 'salt')).not.toBe(mine)
      // The salt is a secret and rotating it is the break-glass. An id that
      // ignored it could not be revoked at all.
      expect(await anonymousId('user_1', 'another salt')).not.toBe(mine)
    })
  })

  it('carries nothing of the account it stands for', async () => {
    const id = await anonymousId('user_01JQZX4N8V9WQKPS', 'salt')
    expect(id).not.toContain('user_')
    expect(id).not.toContain('01JQZX4N8V9WQKPS')
    // Twelve bytes of the MAC, hex. Enough to count distinct users and short
    // enough that it is not a place to hide anything else.
    expect(id).toMatch(/^[0-9a-f]{24}$/)
  })
})

describe('what one recorded event carries', () => {
  /// The binding, as an object that keeps what it was handed.
  function collector(): { points: any[]; writeDataPoint(point: any): void } {
    const points: any[] = []
    return { points, writeDataPoint: (point: any) => points.push(point) }
  }

  it('indexes on the anonymous id, because that is what the query groups by', async () => {
    const metrics = collector()
    await record(metrics, 'salt', 'signed_in', 'user_1', {})

    expect(metrics.points.length).toBe(1)
    expect(metrics.points[0].indexes).toEqual([await anonymousId('user_1', 'salt')])
    // Said as the property as well as the value: nothing in the point is the
    // account id, so there is nothing here for a deletion request to sweep.
    expect(JSON.stringify(metrics.points[0])).not.toContain('user_1')
  })

  it('names the event and the platform, and nothing else', async () => {
    const metrics = collector()
    await record(metrics, 'salt', 'notification_sent', 'user_1', { platform: 'apns', ok: true })
    await record(metrics, 'salt', 'device_registered', 'user_1', {})

    expect(metrics.points[0].blobs).toEqual(['notification_sent', 'apns'])
    // An absent platform is the empty string rather than a hole, because the
    // blobs are positional and a shorter one would shift every column after it.
    expect(metrics.points[1].blobs).toEqual(['device_registered', ''])
  })

  it('counts a failure as 0 and everything else as 1', async () => {
    // `ok === false` and not `!ok`. Most events carry no outcome at all —
    // `signed_in`, `daemon_paired` — and reading an absent one as a failure
    // would put the whole product's success rate at zero.
    const metrics = collector()
    await record(metrics, 'salt', 'notification_failed', 'user_1', { platform: 'apns', ok: false })
    await record(metrics, 'salt', 'notification_sent', 'user_1', { platform: 'apns', ok: true })
    await record(metrics, 'salt', 'signed_in', 'user_1')

    expect(metrics.points.map(point => point.doubles)).toEqual([[0], [1], [1]])
  })

  it('gives one account one index however many events it produces', async () => {
    // The half that makes `count(distinct index1)` a user count rather than an
    // event count.
    const metrics = collector()
    await record(metrics, 'salt', 'signed_in', 'user_1')
    await record(metrics, 'salt', 'device_registered', 'user_1', { platform: 'apns' })
    await record(metrics, 'salt', 'daemon_paired', 'user_2')

    expect(metrics.points[0].indexes).toEqual(metrics.points[1].indexes)
    expect(metrics.points[2].indexes).not.toEqual(metrics.points[0].indexes)
  })
})

// MARK: - The one secret whose wrongness is invisible

/// A relay holding another channel's APNs topic.
///
/// There is one relay per channel and `apns-topic` must equal the receiving
/// app's bundle identifier, which differs per channel. Get it wrong and APNs
/// rejects every push for a token/topic mismatch — `sendApns` returns false and
/// the daemon is told only that a notification "failed". Six secrets have to be
/// right per environment; this is the one nothing would report.
describe('topic and channel must agree', () => {
  it('accepts a topic wearing its own channel suffix', () => {
    expect(topicMismatch({ CHANNEL: 'canary', APNS_TOPIC: 'com.farcooler.ios.canary' })).toBeNull()
    expect(topicMismatch({ CHANNEL: 'preview', APNS_TOPIC: 'com.farcooler.ios.preview' })).toBeNull()
    expect(topicMismatch({ CHANNEL: 'stable', APNS_TOPIC: 'com.farcooler.ios' })).toBeNull()
  })

  it('catches the exact misconfiguration this guards', () => {
    // The canary relay provisioned by copying the stable secrets, which is how
    // this actually happens.
    const problem = topicMismatch({ CHANNEL: 'canary', APNS_TOPIC: 'com.farcooler.ios' })
    expect(problem).toContain('canary')
    expect(problem).toContain('com.farcooler.ios')
  })

  it("catches a stable relay wearing another channel's suffix", () => {
    expect(topicMismatch({ CHANNEL: 'stable', APNS_TOPIC: 'com.farcooler.ios.preview' })).toContain(
      'stable',
    )
  })

  it('says so when the channel itself is not one it knows', () => {
    expect(topicMismatch({ CHANNEL: 'beta', APNS_TOPIC: 'com.farcooler.ios.beta' })).toContain(
      'beta',
    )
  })

  it('stays quiet when there is nothing to compare', () => {
    // A deployment made before this check declared no channel, and every one of
    // those is the stable relay. Refusing on absence would take push down on
    // the one channel that must never lose it.
    expect(topicMismatch({ APNS_TOPIC: 'com.farcooler.ios' })).toBeNull()
    expect(topicMismatch({ CHANNEL: 'stable' })).toBeNull()
    expect(topicMismatch({})).toBeNull()
  })

  // MARK: - And what the route does about it

  /// The function above was tested and the ARM that calls it never ran.
  ///
  /// `topicMismatch` returns early when either half is absent, and this suite
  /// declared no `CHANNEL` at all — so every request through every route in this
  /// file went past `notify`'s misconfiguration check without evaluating it, and
  /// the 500 could be deleted with all 169 tests green. The binding is declared
  /// now, at the correct pairing, and the wrong one is handed in per request.
  describe('and the notify route says so out loud', () => {
    async function fleet() {
      await register('user_1', { liveActivityStartToken: 'start-token' })
      await pair('user_1', 'mine')
    }

    it('refuses to deliver, with a 500 that names what is wrong', async () => {
      // The canary relay provisioned by copying the stable secrets, which is how
      // this actually happens.
      watchFetch()
      await fleet()

      const response = await postAs(
        { CHANNEL: 'canary' },
        '/v1/notify',
        { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
        'mine',
      )

      // 500 and not 400: nothing is wrong with what the machine asked for.
      expect(response.status).toBe(500)
      const body = await response.json<any>()
      expect(body.error).toBe('relay misconfigured')
      expect(body.detail).toContain('canary')
      expect(body.detail).toContain('com.farcooler.ios')
    })

    it('refuses before it has read a single device', async () => {
      // Delivering to none of them and calling it a delivery is the failure this
      // is preventing, so nothing may go out and nothing may be recorded: no
      // push, no roster row, and no `delivered` count for the daemon to believe.
      const calls = watchFetch()
      await fleet()
      const events = watchMetrics()

      const response = await postAs(
        { CHANNEL: 'preview' },
        '/v1/notify',
        { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
        'mine',
      )

      expect(response.status).toBe(500)
      expect(await response.json<any>()).not.toHaveProperty('delivered')
      expect(pushes(calls)).toEqual([])
      expect(events).toEqual([])
      expect(await roster('user_1')).toEqual([])
      expect(await cardOf('user_1')).toBe(null)
    })

    it('delivers on a relay whose topic does belong to its channel', async () => {
      // The other half, without which the test above passes against a route that
      // refuses everything. Both channels here are wrong for THIS suite's
      // `APNS_TOPIC` and right for their own.
      const calls = watchFetch()
      await fleet()

      for (const [channel, topic] of [
        ['canary', 'com.farcooler.ios.canary'],
        ['stable', 'com.farcooler.ios'],
      ]) {
        const response = await postAs(
          { CHANNEL: channel, APNS_TOPIC: topic },
          '/v1/notify',
          { title: 'hi' },
          'mine',
        )
        expect(await response.json()).toEqual({ delivered: 1 })
      }
      expect(pushes(calls).length).toBe(2)
      expect(pushes(calls)[0].headers['apns-topic']).toBe('com.farcooler.ios.canary')
    })

    it('is not what a deployment made before the check gets', async () => {
      // Every relay deployed without a channel is the stable one, and refusing
      // on absence would take push down on the one channel that must never lose
      // it. The unit test above says the function stays quiet; this says the
      // route does.
      const calls = watchFetch()
      await fleet()

      const response = await postAs({ CHANNEL: undefined }, '/v1/notify', { title: 'hi' }, 'mine')

      expect(await response.json()).toEqual({ delivered: 1 })
      expect(pushes(calls).length).toBe(1)
    })
  })
})
