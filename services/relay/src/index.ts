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
      switch (url.pathname) {
        case '/v1/auth/token':
          return await exchangeCode(request, env)
        case '/v1/auth/refresh':
          return await refreshSession(request, env)
        case '/v1/devices':
          return await registerDevice(request, env)
        case '/v1/account':
          return await listAccount(request, env)
        case '/v1/devices/revoke':
          return await revokeDevice(request, env)
        case '/v1/daemons':
          return await pairDaemon(request, env)
        case '/v1/daemons/revoke':
          return await revokeDaemon(request, env)
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
    // WorkOS's own reason is safe to pass through here and is the difference
    // between "sign-in failed" and "your session expired, sign in again".
    const detail = await response.text()
    console.error('workos authenticate failed', response.status, detail)
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

  const body = await request.json<{ platform: string; pushToken: string; label?: string }>()
  if (body.platform !== 'apns' && body.platform !== 'fcm') return json({ error: 'platform' }, 400)
  if (!body.pushToken) return json({ error: 'pushToken' }, 400)

  await env.DB.prepare(
    `INSERT INTO devices (id, account_id, platform, push_token, label, updated_at)
     VALUES (?, ?, ?, ?, ?, ?)
     ON CONFLICT (platform, push_token)
     DO UPDATE SET account_id = excluded.account_id,
                   label = excluded.label,
                   updated_at = excluded.updated_at`,
  )
    .bind(crypto.randomUUID(), account, body.platform, body.pushToken, body.label ?? 'Device', Date.now())
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

  const devices = await env.DB.prepare(
    `SELECT id, platform, label, updated_at FROM devices WHERE account_id = ? ORDER BY updated_at DESC`,
  )
    .bind(account)
    .all<{ id: string; platform: string; label: string; updated_at: number }>()

  const daemons = await env.DB.prepare(
    `SELECT id, label, created_at, last_seen_at FROM daemons WHERE account_id = ? ORDER BY created_at DESC`,
  )
    .bind(account)
    .all<{ id: string; label: string; created_at: number; last_seen_at: number | null }>()

  return json({
    email: null,
    devices: (devices.results ?? []).map(d => ({
      id: d.id,
      platform: d.platform,
      label: d.label,
      updatedAt: d.updated_at,
    })),
    machines: (daemons.results ?? []).map(d => ({
      id: d.id,
      label: d.label,
      createdAt: d.created_at,
      lastSeenAt: d.last_seen_at,
    })),
  })
}

/// Stop notifying a device.
///
/// Scoped by account in the WHERE clause, not checked before it: a delete that
/// verifies ownership in a separate query has a window between the two, and
/// there is no reason to have the window.
async function revokeDevice(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const body = await request.json<{ id: string }>()
  await env.DB.prepare(`DELETE FROM devices WHERE id = ? AND account_id = ?`)
    .bind(body.id, account)
    .run()
  return json({ ok: true })
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

  await env.DB.prepare(
    `INSERT INTO daemons (id, account_id, token_hash, label, created_at) VALUES (?, ?, ?, ?, ?)`,
  )
    .bind(crypto.randomUUID(), account, await sha256(token), body.label ?? 'Machine', Date.now())
    .run()

  await record(env.METRICS, env.ANALYTICS_SALT, 'daemon_paired', account)
  return json({ token })
}

async function revokeDaemon(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const body = await request.json<{ id: string }>()
  await env.DB.prepare(`DELETE FROM daemons WHERE id = ? AND account_id = ?`)
    .bind(body.id, account)
    .run()
  return json({ ok: true })
}

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

  const daemon = await env.DB.prepare(
    `SELECT id, account_id FROM daemons WHERE token_hash = ?`,
  )
    .bind(await sha256(token))
    .first<{ id: string; account_id: string }>()
  if (!daemon) return json({ error: 'unauthorised' }, 401)

  const body = await request.json<{ title: string; subtitle?: string; terminal?: string }>()
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

  await env.DB.prepare(`UPDATE daemons SET last_seen_at = ? WHERE id = ?`)
    .bind(Date.now(), daemon.id)
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
