import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { fingerprintOf, parseEd25519 } from '../src/keys'
import { topicMismatch } from '../src/push'
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
async function signTestJwt(claims: Record<string, unknown>): Promise<string> {
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const payload = base64Url(JSON.stringify(claims))
  const signature = await crypto.subtle.sign(
    'RSASSA-PKCS1-v1_5',
    signing.privateKey,
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
  const now = seconds()
  return await signTestJwt({
    sub: userId,
    email: `${userId}@example.test`,
    iss: ISSUER,
    client_id: CLIENT_ID,
    sid: 'session_test',
    iat: now,
    exp: now + 3600,
  })
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
  return worker.fetch(
    new Request(`https://relay.test${path}`, {
      method: 'POST',
      headers: bearer ? { authorization: `Bearer ${bearer}` } : {},
      body: JSON.stringify(body),
    }),
    env as never,
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

beforeEach(async () => {
  for (const table of ['live_activities', 'devices', 'daemons', 'accounts']) {
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
    const now = seconds()
    return {
      sub: 'user_1',
      email: 'user_1@example.test',
      iss: ISSUER,
      client_id: CLIENT_ID,
      sid: 'session_test',
      iat: now,
      exp: now + 3600,
      ...overrides,
    }
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
      { title: 'cursor failed', subtitle: "Its last turn didn't finish", terminal: 'term-1', status: 'done', label: 'cursor', failed: true },
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

describe('the signed-in routes', () => {
  it('refuse a caller with no session', async () => {
    const paths = [
      '/v1/devices',
      '/v1/devices/activity',
      '/v1/devices/lookup',
      '/v1/devices/verify',
      '/v1/daemons',
      '/v1/account',
      '/v1/devices/revoke',
    ]
    for (const path of paths) {
      expect((await post(path, {})).status, path).toBe(401)
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

    const row = await env.DB.prepare(`SELECT * FROM live_activities`).first<any>()
    expect(row?.terminal).toBe('term-1')
    expect(row?.update_token).toBe('update-token')
    expect(row?.environment).toBe('development')
  })

  it('replaces the token when the same terminal starts another activity', async () => {
    // APNs issues a fresh update token per activity and the old one is dead, so
    // a second row would be a card nobody can reach.
    watchFetch()
    const session = await sessionFor('user_1')
    await post('/v1/devices/activity', { terminal: 'term-1', updateToken: 'first' }, session)
    await post('/v1/devices/activity', { terminal: 'term-1', updateToken: 'second' }, session)

    const rows = await env.DB.prepare(`SELECT update_token FROM live_activities`).all<any>()
    expect(rows.results?.length).toBe(1)
    expect(rows.results?.[0].update_token).toBe('second')
  })

  it('forgets the activity when the app says it is over', async () => {
    watchFetch()
    const session = await sessionFor('user_1')
    await post('/v1/devices/activity', { terminal: 'term-1', updateToken: 'update' }, session)
    await post('/v1/devices/activity', { terminal: 'term-1', updateToken: null }, session)

    const rows = await env.DB.prepare(`SELECT id FROM live_activities`).all<any>()
    expect(rows.results?.length).toBe(0)
  })

  it('needs to know which terminal', async () => {
    // The row is keyed on it. Without one, every agent on the account collapses
    // into a single card that disagrees with all of them.
    watchFetch()
    expect(
      (await post('/v1/devices/activity', { updateToken: 'u' }, await sessionFor('user_1'))).status,
    ).toBe(400)
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
    expect(aps['content-state']).toEqual({
      status: 'blocked',
      detail: 'Waiting for your answer',
    })
    // A fixed contract with the app: this string names the Swift type.
    expect(aps['attributes-type']).toBe('AgentActivityAttributes')
    expect(aps.attributes).toEqual({
      terminal: 'term-1',
      label: 'refactor-auth',
      machine: 'Studio',
    })
    // Stale after an hour, never dismissed. Nothing reports an update token for
    // a card the relay started while the app was closed, so `done` can arrive
    // to find no row and the card would otherwise claim "Needs You" forever. A
    // dismissal date instead would delete the notification the product exists
    // for, in the case where the agent really is still blocked.
    expect(aps['stale-date']).toBe(aps.timestamp + 3600)
    expect(aps['dismissal-date']).toBeUndefined()
  })

  it('puts the turn clock in the attributes of the card it starts', async () => {
    // The card counts elapsed time from this and nothing else: iOS renders a
    // date as a native timer, so there is no push per tick and no state to
    // update. Drop it and the timer is simply absent — no error, no failed
    // delivery, nothing in a log to find.
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
    expect(activity.body.aps.attributes).toEqual({
      terminal: 'term-1',
      label: 'claude',
      machine: 'Studio',
      startedAt: 1_755_000_000_000,
    })
    // A NUMBER, in milliseconds. The app's decoder tells seconds from
    // milliseconds apart by magnitude and reads a string back as nil, which
    // costs the timer without costing the card — the one failure mode that
    // reports itself nowhere.
    expect(typeof activity.body.aps.attributes.startedAt).toBe('number')
  })

  it('sends no clock at all for a machine that named none', async () => {
    // Every field here is optional forever. A daemon older than `startedAt`
    // must still start a card, and a card with no timer is the intended
    // outcome — unlike a zero, which would count up from January 1970.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')

    const [, activity] = pushes(calls)
    expect(activity.body.aps.attributes).toEqual({
      terminal: 'term-1',
      label: 'claude needs you',
      machine: 'Studio',
    })
    expect('startedAt' in activity.body.aps.attributes).toBe(false)
  })

  it('never repeats the turn clock on an update or an end', async () => {
    // Attributes are the activity's identity, fixed for its whole life, and
    // APNs REJECTS a push that repeats them. A `startedAt` leaking onto either
    // of these does not move a timer — it costs the update entirely, so the
    // card freezes on whatever it last said and looks like a dead relay.
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
      // Nor smuggled into the state, which is the app's `ContentState` and
      // has no such property: an extra key there fails to decode and takes
      // the whole update down with it.
      expect(state.startedAt).toBeUndefined()
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
    expect(activity.body.aps['content-state']).toEqual({ status: 'done', detail: 'Tests pass' })
    expect(activity.body.aps['dismissal-date']).toBeGreaterThan(1_600_000_000)
    expect(activity.body.aps.attributes).toBeUndefined()

    const rows = await env.DB.prepare(`SELECT id FROM live_activities`).all<any>()
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
      `INSERT INTO live_activities (id, account_id, terminal, update_token, updated_at)
       VALUES (?, ?, ?, ?, ?)`,
    )
      .bind(crypto.randomUUID(), 'user_2', 'term-1', 'their-update-token', Date.now())
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
    expect(sent[0].body.aps['content-state']).toEqual({
      status: 'working',
      detail: '3/7 · Designing test matrix',
    })
    // No alert dictionary: an activity push carrying one is PRESENTED, which is
    // the banner this tier must never produce.
    expect(sent[0].body.aps.alert).toBeUndefined()
    // Priority 10 spends the app's Live Activity budget, and it is the same
    // budget the blocked alert depends on.
    expect(sent[0].headers['apns-priority']).toBe('5')
  })

  it('starts a card, silently, for an agent that has begun working', async () => {
    // The card follows a WHOLE RUN, so it has to exist while the agent is
    // merely busy — that is the only stretch there is anything to watch. It
    // starts from the push-to-start token because nothing on the phone is awake
    // to start it.
    const calls = watchFetch()
    await ready()
    await post(
      '/v1/notify',
      { title: 'claude', subtitle: 'Reading watch.rs', terminal: 'term-1', status: 'working' },
      'mine',
    )

    // One push, and it is the card. A working state never sends an alert push
    // either, so there is nothing before it.
    const [activity, ...rest] = pushes(calls)
    expect(rest).toEqual([])
    expect(activity.url).toContain('/device/start-token')
    expect(activity.headers['apns-push-type']).toBe('liveactivity')
    expect(activity.body.aps.event).toBe('start')
    expect(activity.body.aps['content-state']).toEqual({
      status: 'working',
      detail: 'Reading watch.rs',
    })

    // The one thing this must never do. An activity push carrying an alert
    // dictionary is PRESENTED — lock screen banner, Apple Watch haptic — so a
    // start that carried one would buzz the wrist every time any agent picked
    // up work. Silent creation is the whole difference between a card and an
    // interruption.
    expect(activity.body.aps.alert).toBeUndefined()
    // And at the working priority, because priority 10 spends the app's Live
    // Activity budget — the same budget the blocked alert depends on.
    expect(activity.headers['apns-priority']).toBe('5')
  })

  it('gives the card it starts for a working agent its turn clock', async () => {
    // Attributes are fixed for an activity's life, so this start is the only
    // push that can carry the timer. A whole-run card without it counts nothing
    // for as long as it exists.
    const calls = watchFetch()
    await ready()
    await post(
      '/v1/notify',
      {
        title: 'claude',
        subtitle: 'Writing fruit.txt',
        terminal: 'term-1',
        status: 'working',
        label: 'claude',
        startedAt: 1_755_000_000_000,
      },
      'mine',
    )

    const [activity] = pushes(calls)
    expect(activity.body.aps['attributes-type']).toBe('AgentActivityAttributes')
    expect(activity.body.aps.attributes).toEqual({
      terminal: 'term-1',
      label: 'claude',
      machine: 'Studio',
      startedAt: 1_755_000_000_000,
    })
    // A NUMBER, in milliseconds. A string reads back as nil in the app and
    // costs the timer without costing the card.
    expect(typeof activity.body.aps.attributes.startedAt).toBe('number')
  })

  it('starts ONE card for a run, however many working pushes arrive', async () => {
    // The failure this row exists to prevent, and the worst one this feature
    // could ship with. Only the app can report an update token, and the app may
    // never run for the whole length of a run — so without a row written at
    // start time, every `working` push finds nothing running and starts ANOTHER
    // card. The daemon sends one about every ten seconds: a half-hour run would
    // leave on the order of a hundred and eighty cards stacked on the lock
    // screen, and the relay holds no token for a single one of them, so nothing
    // it can ever do will take them back.
    const calls = watchFetch()
    await ready()
    for (const detail of ['Reading watch.rs', 'Editing watch.rs', 'Running tests']) {
      await post(
        '/v1/notify',
        { title: 'claude', subtitle: detail, terminal: 'term-1', status: 'working' },
        'mine',
      )
    }

    const sent = pushes(calls)
    expect(sent.length).toBe(1)
    expect(sent[0].body.aps.event).toBe('start')
    expect(sent[0].url).toContain('/device/start-token')

    // One row, holding the sentinel: a card is running for this agent and
    // nothing yet knows where. It is what the UNIQUE (account_id, terminal)
    // constraint refuses the second start against.
    const rows = await env.DB.prepare(
      `SELECT terminal, update_token FROM live_activities`,
    ).all<any>()
    expect(rows.results).toEqual([{ terminal: 'term-1', update_token: '' }])
  })

  it('updates the card it started blind once the app reports its token', async () => {
    // Recovery, and it needs nothing from anybody: iOS replays running
    // activities to the app at launch, the app files the update token for a card
    // it never started, and the row the relay wrote blind fills in. From that
    // moment the card is addressable for the rest of the run.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
    await running('term-1')
    await post(
      '/v1/notify',
      { title: 'claude', subtitle: 'Running tests', terminal: 'term-1', status: 'working' },
      'mine',
    )

    const [start, update, ...rest] = pushes(calls)
    expect(rest).toEqual([])
    expect(start.body.aps.event).toBe('start')
    expect(update.url).toContain('/device/update-token')
    expect(update.body.aps.event).toBe('update')
    expect(update.body.aps['content-state']).toEqual({
      status: 'working',
      detail: 'Running tests',
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
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
    await post(
      '/v1/notify',
      { title: 'claude needs you', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    const [start, alert] = pushes(calls)
    expect(start.body.aps.event).toBe('start')
    expect(alert.headers['apns-push-type']).toBe('alert')
    expect(alert.url).toContain('/device/device-token')
  })

  it('forgets a card it cannot address when the agent finishes', async () => {
    // There is nowhere to send the end — only the app could have told the relay
    // where — so it sends nothing rather than pushing at the empty string and
    // counting a delivery that cannot have happened. The row goes anyway: an
    // update token dies with its activity, and a row kept past the run would
    // refuse this terminal a card for every run after it. The abandoned card
    // clears itself on the `stale-date` its start carried.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
    await post('/v1/notify', { title: 'Done', terminal: 'term-1', status: 'done' }, 'mine')

    const activities = pushes(calls).filter(call => call.body.aps?.event)
    expect(activities.map(call => call.body.aps.event)).toEqual(['start'])

    const rows = await env.DB.prepare(`SELECT id FROM live_activities`).all<any>()
    expect(rows.results?.length).toBe(0)

    // And the next run gets its card, because the row it would have collided
    // with is gone.
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(2)
  })

  it('still alerts and starts a card when the agent goes on to block', async () => {
    // The tier above working is unchanged: a banner, at priority 10, and a card
    // started from the push-to-start token.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude needs you', terminal: 'term-1', status: 'blocked' }, 'mine')

    const [alert, activity] = pushes(calls)
    expect(alert.headers['apns-push-type']).toBe('alert')
    expect(activity.body.aps.event).toBe('start')
    expect(activity.headers['apns-priority']).toBe('10')
    expect(activity.body.aps.alert).toEqual({ title: 'claude needs you', body: '' })
  })

  it('replaces a card stuck on Working when the agent blocks', async () => {
    // The product's primary scenario, on a phone whose app has not run. The
    // working push claimed the row with the sentinel, so the blocked push found
    // a card it could not address and left it — the lock screen read "Working"
    // beside a banner saying the agent needed an answer, until the run ended or
    // the hour-long stale date passed. A card that states the wrong thing there
    // is worse than a duplicate: the old one is left to expire and the app ends
    // it the next time it runs.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
    await post(
      '/v1/notify',
      { title: 'claude needs you', subtitle: 'Create haiku.txt?', terminal: 'term-1', status: 'blocked' },
      'mine',
    )

    const starts = pushes(calls).filter(call => call.body.aps?.event === 'start')
    expect(starts.length).toBe(2)
    expect(starts[1].body.aps['content-state']).toEqual({
      status: 'blocked',
      detail: 'Create haiku.txt?',
    })
    // And it is presented, unlike the silent start before it.
    expect(starts[1].body.aps.alert).toEqual({
      title: 'claude needs you',
      body: 'Create haiku.txt?',
    })
    // The row remembers the tier it is now showing, which is what keeps this to
    // one replacement.
    const row = await env.DB.prepare(`SELECT blind_status FROM live_activities`).first<any>()
    expect(row?.blind_status).toBe('blocked')
  })

  it('replaces it once, however many times the agent blocks', async () => {
    // Without the memory on the row, every blocked push while unaddressable
    // would stack another card — the same failure `TOKEN_UNKNOWN` was written to
    // prevent, one tier up.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')
    for (const question of ['Create haiku.txt?', 'Delete build/?', 'Force push?']) {
      await post(
        '/v1/notify',
        { title: 'claude needs you', subtitle: question, terminal: 'term-1', status: 'blocked' },
        'mine',
      )
    }

    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(2)
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
    const row = await env.DB.prepare(`SELECT update_token, dismissed_at FROM live_activities`)
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

  it('forgets a dismissal that has outlived the card it was about', async () => {
    // `done` deletes the row with the run, but a run can end without one — a
    // daemon killed mid-turn. A row kept forever would refuse this terminal a
    // card for every run after it, silently and permanently.
    const calls = watchFetch()
    await ready()
    await running('term-1')
    await post(
      '/v1/devices/activity',
      { terminal: 'term-1', updateToken: null, dismissed: true },
      await sessionFor('user_1'),
    )
    await env.DB.prepare(`UPDATE live_activities SET dismissed_at = ?`)
      .bind(Date.now() - 2 * 60 * 60 * 1000)
      .run()
    await post('/v1/notify', { title: 'claude', terminal: 'term-1', status: 'working' }, 'mine')

    expect(pushes(calls).filter(call => call.body.aps?.event === 'start').length).toBe(1)
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

    const rows = await env.DB.prepare(`SELECT id FROM live_activities`).all<any>()
    expect(rows.results?.length).toBe(0)
  })

  it('clears the memory of a dismissal once the app files a real token', async () => {
    // A real update token means there is a card, it is up, and it can be moved
    // in place. Anything the relay remembered about not being able to reach one
    // is answered by that.
    watchFetch()
    const session = await sessionFor('user_1')
    await ready()
    await running('term-1')
    await post('/v1/devices/activity', { terminal: 'term-1', updateToken: null, dismissed: true }, session)
    await running('term-1', 'fresh-token')

    const row = await env.DB.prepare(
      `SELECT update_token, blind_status, dismissed_at FROM live_activities`,
    ).first<any>()
    expect(row?.update_token).toBe('fresh-token')
    expect(row?.blind_status).toBe(null)
    expect(row?.dismissed_at).toBe(null)
  })

  it('leaves activities alone when the daemon names no terminal', async () => {
    // A row keyed on an empty terminal is every agent on the account sharing
    // one card. Better no activity than one that lies about all of them.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'hi', status: 'blocked' }, 'mine')

    expect(pushes(calls).length).toBe(1)
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

/// The SSH wire encoding of an ed25519 public key: two length-prefixed strings,
/// the algorithm name and the 32 bytes. This is what the fingerprint is over,
/// which is why the test builds it rather than hashing the key alone.
function sshBlob(raw: Uint8Array): Uint8Array {
  const name = new TextEncoder().encode('ssh-ed25519')
  const out = new Uint8Array(4 + name.length + 4 + raw.length)
  new DataView(out.buffer).setUint32(0, name.length)
  out.set(name, 4)
  new DataView(out.buffer).setUint32(4 + name.length, raw.length)
  out.set(raw, 8 + name.length)
  return out
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
    // The algorithm inside the blob, not the label in front of it. Text anyone
    // can write is not what a verifier goes by.
    expect(parseEd25519(line.replace('ssh-ed25519 ', 'ssh-rsa '))).toBe(null)
    expect(parseEd25519('ssh-ed25519 not-base64')).toBe(null)
    expect(parseEd25519('')).toBe(null)
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
})
