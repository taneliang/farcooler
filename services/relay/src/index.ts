/// The relay's routes.
///
/// Three groups, and the asymmetry between them is the whole security model.
/// Signing in exchanges a WorkOS code for a session. Registering a device and
/// pairing a machine are done BY A SIGNED-IN PERSON. Sending a notification is
/// done by a machine holding a token that person issued — so a machine can only
/// ever notify the account that paired it. See `/v1/notify`.
///
/// The apps hold a WorkOS client id, which is public by design, and never an API
/// key. The code exchange happens here because that is the one step that needs
/// the secret, and a secret in a repo that is about to be open source — or in an
/// app bundle anyone can unzip — is not a secret.

import { record, type Metrics } from './analytics'
import { verifySession } from './workos'
import { sendPush } from './push'

export interface Env {
  DB: D1Database
  /// Cloudflare's rate-limiting binding, guarding the two routes that spend the
  /// relay's WorkOS API key on an anonymous caller's behalf. Optional in the
  /// type so a `wrangler dev` without the binding still runs — see `withinRate`,
  /// which fails OPEN for exactly that reason and says so.
  AUTH_LIMIT?: RateLimit
  METRICS: Metrics
  WORKOS_CLIENT_ID: string
  WORKOS_API_KEY: string
  ANALYTICS_SALT: string
  APNS_KEY_P8: string
  APNS_KEY_ID: string
  APNS_TEAM_ID: string
  APNS_TOPIC: string
  FCM_SERVICE_ACCOUNT: string
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url)
    if (request.method !== 'POST') return json({ error: 'method' }, 405)

    try {
      // The unauthenticated routes, throttled before they do any work.
      //
      // These three are the only ones anyone can call without a credential, and
      // two of them spend the relay's WorkOS API key per request: unlimited,
      // they are a way to exhaust the upstream rate limit and deny sign-in to
      // every real user, at no cost to the caller. Everything below them
      // requires a session or a daemon token and is throttled by having to have
      // one.
      if (url.pathname.startsWith('/v1/auth/') && !(await withinRate(request, env))) {
        return json({ error: 'slow down' }, 429)
      }

      switch (url.pathname) {
        case '/v1/auth/token':
          return await exchangeCode(request, env)
        case '/v1/auth/refresh':
          return await refreshSession(request, env)
        case '/v1/auth/logout':
          return await logout(request, env)
        case '/v1/devices':
          return await registerDevice(request, env)
        case '/v1/account':
          return await listAccount(request, env)
        case '/v1/devices/revoke':
          return await revokeOwned(request, env, 'devices')
        case '/v1/daemons':
          return await pairDaemon(request, env)
        case '/v1/daemons/revoke':
          return await revokeOwned(request, env, 'daemons')
        case '/v1/notify':
          return await notify(request, env)
        default:
          return json({ error: 'not found' }, 404)
      }
    } catch (error) {
      // Never the message: an error from D1 or WorkOS can carry a query or a
      // token fragment, and this response goes to whoever asked.
      console.error(error)
      return json({ error: 'internal' }, 500)
    }
  },
}

// MARK: - Signing in

/// Trade an authorisation code for a session.
///
/// PKCE, so the code is worthless to anything that did not start the sign-in:
/// the app invented a verifier, sent only its hash to WorkOS, and proves
/// possession here. That matters because the redirect comes back through a
/// custom URL scheme, which any app on the device can claim.
async function exchangeCode(request: Request, env: Env): Promise<Response> {
  const body = await request.json<{ code: string; verifier: string }>()
  if (!body.code || !body.verifier) return json({ error: 'code' }, 400)

  return await workosToken(env, {
    grant_type: 'authorization_code',
    code: body.code,
    code_verifier: body.verifier,
  })
}

/// Trade a refresh token for a fresh session.
///
/// Kept server-side for the same reason as the exchange: WorkOS wants the API
/// key on this call too, and an app that could refresh on its own would be an
/// app carrying that key.
async function refreshSession(request: Request, env: Env): Promise<Response> {
  const body = await request.json<{ refreshToken: string }>()
  if (!body.refreshToken) return json({ error: 'refreshToken' }, 400)

  return await workosToken(env, {
    grant_type: 'refresh_token',
    refresh_token: body.refreshToken,
  })
}

/// End a session at WorkOS, not merely on the device.
///
/// Clearing tokens locally left the refresh token valid upstream until natural
/// expiry, so anyone who had lifted it kept minting sessions after the user
/// believed they had signed out. Unauthenticated on purpose: possession of the
/// refresh token IS the authorisation, and requiring a valid access token would
/// mean the one case that most needs to work — a session already going wrong —
/// is the one that cannot.
async function logout(request: Request, env: Env): Promise<Response> {
  const body = await request.json<{ refreshToken?: unknown }>()
  if (typeof body.refreshToken !== 'string' || !body.refreshToken) {
    return json({ error: 'refreshToken' }, 400)
  }

  await fetch('https://api.workos.com/user_management/sessions/logout', {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      authorization: `Bearer ${env.WORKOS_API_KEY}`,
    },
    body: JSON.stringify({ session_id: body.refreshToken }),
  }).catch(() => undefined)

  // Always ok. The device is clearing its copy either way, and an error here
  // would only teach the app to leave a credential in place when the server is
  // having a bad day.
  return json({ ok: true })
}

/// The one call that needs the API key, in the one place that has it.
async function workosToken(env: Env, fields: Record<string, string>): Promise<Response> {
  const response = await fetch('https://api.workos.com/user_management/authenticate', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...fields,
      client_id: env.WORKOS_CLIENT_ID,
      client_secret: env.WORKOS_API_KEY,
    }),
  })

  if (!response.ok) {
    // The STATUS and WorkOS's own error code, never the body. A 4xx body for a
    // failed `refresh_token` or `authorization_code` grant can echo the
    // submitted credential straight back, and Cloudflare's logs are a lower
    // trust boundary than the secret store the API key lives in. The top-level
    // handler already refuses to log error text for exactly this reason; this
    // path used to contradict it.
    const code = await response
      .json<{ error?: string }>()
      .then(body => body.error ?? 'unknown')
      .catch(() => 'unparseable')
    console.error('workos authenticate failed', response.status, code)
    return json({ error: 'auth', status: response.status }, 401)
  }

  const session = await response.json<{
    access_token: string
    refresh_token: string
    user?: { id?: string; email?: string }
  }>()

  // Create the account here as well as in `requireAccount`, so a person who
  // signs in and never registers a device still exists — that is the marketing
  // and billing record, and it should not depend on a push permission prompt.
  if (session.user?.id) {
    await env.DB.prepare(
      `INSERT INTO accounts (id, created_at, email) VALUES (?, ?, ?)
       ON CONFLICT (id) DO UPDATE SET email = excluded.email`,
    )
      .bind(session.user.id, Date.now(), session.user.email ?? null)
      .run()
    await record(env.METRICS, env.ANALYTICS_SALT, 'signed_in', session.user.id)
  }

  return json({
    accessToken: session.access_token,
    refreshToken: session.refresh_token,
    userId: session.user?.id ?? '',
    email: session.user?.email ?? '',
  })
}

// MARK: - Signed-in routes

/// Remember where to reach this device.
///
/// Upserted on the push token rather than the device id, because Apple and
/// Google reissue tokens freely — the same phone coming back with a new token
/// is one device, and a row per token would fan a notification out to a pile of
/// dead addresses.
async function registerDevice(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const body = await request.json<{
    platform: string
    pushToken: string
    label?: string
    version?: string
  }>()
  if (body.platform !== 'apns' && body.platform !== 'fcm') return json({ error: 'platform' }, 400)
  if (!body.pushToken) return json({ error: 'pushToken' }, 400)

  // `version` is optional and stays optional: an App Store build from before
  // this column existed still registers, it just has nothing to report.
  await env.DB.prepare(
    `INSERT INTO devices (id, account_id, platform, push_token, label, version, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (platform, push_token)
     DO UPDATE SET account_id = excluded.account_id,
                   label = excluded.label,
                   version = COALESCE(excluded.version, devices.version),
                   updated_at = excluded.updated_at`,
  )
    .bind(
      crypto.randomUUID(),
      account,
      body.platform,
      body.pushToken,
      body.label ?? 'Device',
      typeof body.version === 'string' ? body.version.slice(0, 64) : null,
      Date.now(),
    )
    .run()

  await record(env.METRICS, env.ANALYTICS_SALT, 'device_registered', account, {
    platform: body.platform,
  })
  return json({ ok: true })
}

/// Everything this account has registered, for the apps' management screen.
///
/// There is no web dashboard and there is not going to be one. Two lists and two
/// delete buttons do not need a second product with its own login, and every
/// person who has this data already has an app open.
///
/// Never the tokens — not the push tokens, not the daemon token hashes. A screen
/// that lists devices needs to name them, not to be able to become them.
async function listAccount(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const who = await env.DB.prepare(`SELECT email FROM accounts WHERE id = ?`)
    .bind(account)
    .first<{ email: string | null }>()

  const devices = await env.DB.prepare(
    `SELECT id, platform, label, version, updated_at FROM devices
     WHERE account_id = ? ORDER BY updated_at DESC`,
  )
    .bind(account)
    .all<{
      id: string
      platform: string
      label: string
      version: string | null
      updated_at: number
    }>()

  const daemons = await env.DB.prepare(
    `SELECT id, label, version, created_at, last_seen_at, expires_at FROM daemons
     WHERE account_id = ? ORDER BY created_at DESC`,
  )
    .bind(account)
    .all<{
      id: string
      label: string
      version: string | null
      created_at: number
      last_seen_at: number | null
      expires_at: number | null
    }>()

  return json({
    email: who?.email ?? null,
    devices: (devices.results ?? []).map(d => ({
      id: d.id,
      platform: d.platform,
      label: d.label,
      version: d.version,
      updatedAt: d.updated_at,
    })),
    machines: (daemons.results ?? []).map(d => ({
      id: d.id,
      label: d.label,
      version: d.version,
      createdAt: d.created_at,
      lastSeenAt: d.last_seen_at,
      expiresAt: d.expires_at,
    })),
  })
}

/// Stop notifying a device, or stop a machine notifying anything.
///
/// One function for both tables. They were two copies of the same eight lines,
/// which meant the missing id guard and the did-anything-happen answer had to be
/// remembered twice.
///
/// Scoped by account in the WHERE clause, not checked before it: a delete that
/// verifies ownership in a separate query has a window between the two, and
/// there is no reason to have the window.
///
/// `ok` reports whether a row actually went. Answering `true` unconditionally
/// made revoking an id belonging to nobody indistinguishable from a real
/// delete — and the app removes the row optimistically on that answer, so a
/// no-op read as success right up until the list reloaded.
async function revokeOwned(
  request: Request,
  env: Env,
  table: 'devices' | 'daemons',
): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const body = await request.json<{ id?: unknown }>()
  // Typed, not merely present: a non-string id reaches the D1 binder and throws,
  // which the top-level catch turns into a 500 for what is a bad request.
  if (typeof body.id !== 'string' || !body.id) return json({ error: 'id' }, 400)

  const result = await env.DB.prepare(
    `DELETE FROM ${table} WHERE id = ? AND account_id = ?`,
  )
    .bind(body.id, account)
    .run()
  return json({ ok: (result.meta?.changes ?? 0) > 0 })
}

/// Issue a machine a token for this account.
///
/// Returned ONCE and stored only as a hash. The phone hands it to the daemon
/// over the ssh channel it already trusts, which is why this can be a plain
/// bearer token rather than something with a key exchange around it.
async function pairDaemon(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const body = await request.json<{ label?: string }>()
  const token = crypto.randomUUID().replaceAll('-', '') + crypto.randomUUID().replaceAll('-', '')

  const now = Date.now()
  await env.DB.prepare(
    `INSERT INTO daemons (id, account_id, token_hash, label, created_at, expires_at)
     VALUES (?, ?, ?, ?, ?, ?)`,
  )
    .bind(
      crypto.randomUUID(),
      account,
      await sha256(token),
      body.label ?? 'Machine',
      now,
      now + TOKEN_LIFETIME_MS,
    )
    .run()

  await record(env.METRICS, env.ANALYTICS_SALT, 'daemon_paired', account)
  return json({ token })
}

/// How long a machine may notify before it must be paired again.
///
/// A year: long enough that re-pairing is rare, short enough that a token
/// nobody is watching stops mattering eventually. Before this, daemon tokens
/// never expired at all, so a single observation was permanent.
const TOKEN_LIFETIME_MS = 365 * 24 * 60 * 60 * 1000

// MARK: - The machine route

/// Notify the account that paired this daemon.
///
/// The request carries NO destination. That is the point: the daemon says what
/// happened, and the relay decides who hears about it, so a stolen daemon token
/// is worth exactly one thing — notifying the phone of the person it was stolen
/// from.
///
/// The body is deliberately thin. A title and a terminal are enough to get
/// someone to open the app, and the relay has no business holding a
/// conversation's contents in transit.
async function notify(request: Request, env: Env): Promise<Response> {
  const header = request.headers.get('authorization') ?? ''
  const token = header.startsWith('Bearer ') ? header.slice(7) : ''
  if (!token) return json({ error: 'unauthorised' }, 401)

  // Expiry checked in the WHERE clause, not after: a token that has run out is
  // a token that does not exist. NULL means a pairing issued before expiries
  // existed, which keeps working — logging those machines out to introduce a
  // policy would break a feature people had just set up.
  const daemon = await env.DB.prepare(
    `SELECT id, account_id FROM daemons
     WHERE token_hash = ? AND (expires_at IS NULL OR expires_at > ?)`,
  )
    .bind(await sha256(token), Date.now())
    .first<{ id: string; account_id: string }>()
  if (!daemon) return json({ error: 'unauthorised' }, 401)

  const body = await request.json<{
    title: string
    subtitle?: string
    terminal?: string
    version?: string
  }>()
  if (!body.title) return json({ error: 'title' }, 400)

  const devices = await env.DB.prepare(
    `SELECT platform, push_token FROM devices WHERE account_id = ?`,
  )
    .bind(daemon.account_id)
    .all<{ platform: string; push_token: string }>()

  let delivered = 0
  for (const device of devices.results ?? []) {
    const ok = await sendPush(env, device.platform, device.push_token, {
      title: body.title,
      subtitle: body.subtitle ?? '',
      terminal: body.terminal ?? '',
    })
    if (ok) delivered += 1
    await record(
      env.METRICS,
      env.ANALYTICS_SALT,
      ok ? 'notification_sent' : 'notification_failed',
      daemon.account_id,
      { platform: device.platform, ok },
    )
  }

  // Version alongside last-seen, and from the same request, because a machine
  // that notifies is a machine that is running — which is exactly when what it
  // is running is worth recording.
  await env.DB.prepare(
    `UPDATE daemons SET last_seen_at = ?, version = COALESCE(?, version) WHERE id = ?`,
  )
    .bind(
      Date.now(),
      typeof body.version === 'string' ? body.version.slice(0, 64) : null,
      daemon.id,
    )
    .run()

  return json({ delivered })
}

// MARK: - Helpers

/// The signed-in account, or the response to send instead.
async function requireAccount(request: Request, env: Env): Promise<string | Response> {
  const header = request.headers.get('authorization') ?? ''
  if (!header.startsWith('Bearer ')) return json({ error: 'unauthorised' }, 401)

  const session = await verifySession(header.slice(7), env)
  if (!session) return json({ error: 'unauthorised' }, 401)

  // First sight of an account creates it. There is no signup step to get wrong
  // and no window where someone is authenticated but has nowhere to be stored.
  await env.DB.prepare(
    `INSERT INTO accounts (id, created_at, email) VALUES (?, ?, ?)
     ON CONFLICT (id) DO UPDATE SET email = excluded.email`,
  )
    .bind(session.userId, Date.now(), session.email ?? null)
    .run()

  return session.userId
}

/// Whether this caller may make another auth request.
///
/// Keyed on the connecting IP, which is the only thing an unauthenticated
/// caller has. Not a strong identity — a botnet has many — but it is what stops
/// one client burning the WorkOS quota, which is the realistic failure.
///
/// Fails OPEN when the binding is absent. A local `wrangler dev` has no rate
/// limiter, and a relay that refused every sign-in because a binding was
/// missing would be a worse outage than the one being prevented. Production
/// declares it in wrangler.toml; if it is ever missing there, sign-in works and
/// the protection is gone, which is the right way round for this specific
/// guard.
async function withinRate(request: Request, env: Env): Promise<boolean> {
  if (!env.AUTH_LIMIT) return true
  const ip = request.headers.get('cf-connecting-ip') ?? 'unknown'
  const { success } = await env.AUTH_LIMIT.limit({ key: ip })
  return success
}

async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))
  return [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2, '0')).join('')
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}
