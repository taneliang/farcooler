/// The relay's routes.
///
/// Four of them, and the asymmetry between the first three and the last is the
/// whole security model: registering a device and pairing a machine are done BY
/// A SIGNED-IN PERSON, and sending a notification is done by a machine that
/// holds a token that person issued. A machine can therefore only ever notify
/// the account that paired it — see `/v1/notify`.

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
        case '/v1/devices':
          return await registerDevice(request, env)
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
