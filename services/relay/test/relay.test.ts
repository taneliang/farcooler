import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'

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

async function sessionFor(userId: string): Promise<string> {
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const claims = base64Url(
    JSON.stringify({
      sub: userId,
      email: `${userId}@example.test`,
      exp: Math.floor(Date.now() / 1000) + 3600,
    }),
  )
  const signature = await crypto.subtle.sign(
    'RSASSA-PKCS1-v1_5',
    signing.privateKey,
    new TextEncoder().encode(`${header}.${claims}`),
  )
  return `${header}.${claims}.${base64Url(new Uint8Array(signature))}`
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

describe('the signed-in routes', () => {
  it('refuse a caller with no session', async () => {
    const paths = [
      '/v1/devices',
      '/v1/devices/activity',
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

  it('leaves activities alone when the daemon names no terminal', async () => {
    // A row keyed on an empty terminal is every agent on the account sharing
    // one card. Better no activity than one that lies about all of them.
    const calls = watchFetch()
    await ready()
    await post('/v1/notify', { title: 'hi', status: 'blocked' }, 'mine')

    expect(pushes(calls).length).toBe(1)
  })
})
