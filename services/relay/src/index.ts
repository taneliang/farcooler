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
import { fingerprintOf, parseEd25519, verifyEd25519 } from './keys'
import { verifySession } from './workos'
import {
  isEnvironment,
  sendLiveActivity,
  sendPush,
  topicMismatch,
  type Activity,
  type ActivityState,
  type Environment,
} from './push'

export interface Env {
  DB: D1Database
  /// Cloudflare's rate-limiting binding, guarding the two routes that spend the
  /// relay's WorkOS API key on an anonymous caller's behalf. Optional in the
  /// type so a `wrangler dev` without the binding still runs — see `withinRate`,
  /// which fails OPEN for exactly that reason and says so.
  AUTH_LIMIT?: RateLimit
  METRICS: Metrics
  WORKOS_CLIENT_ID: string
  /// The `iss` this environment's tokens must carry. Required, not optional:
  /// a missing issuer would have to mean "accept any", and one relay per
  /// channel is precisely the arrangement where the token from the WorkOS
  /// environment next door verifies against nothing else. Set in wrangler.toml
  /// per environment.
  WORKOS_ISSUER: string
  WORKOS_API_KEY: string
  ANALYTICS_SALT: string
  APNS_KEY_P8: string
  APNS_KEY_ID: string
  APNS_TEAM_ID: string
  APNS_TOPIC: string
  /// Which channel this deployment IS, so it can check that the APNs topic
  /// above belongs to it. Set per environment in wrangler.toml; absent on a
  /// deployment made before this check, which is only ever the stable one.
  CHANNEL?: string
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
        case '/v1/devices/activity':
          return await registerActivity(request, env)
        case '/v1/devices/lookup':
          return await lookupDevice(request, env)
        case '/v1/devices/verify':
          return await verifyDevice(request, env)
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

/// Trade an authorization code for a session.
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
/// refresh token IS the authorization, and requiring a valid access token would
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
///
/// And, since the onboarding ceremony, where a device says which key it holds —
/// proved rather than claimed. See `provenFingerprint`.
async function registerDevice(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const body = await request.json<Registration>()
  if (body.platform !== 'apns' && body.platform !== 'fcm') return json({ error: 'platform' }, 400)
  if (!body.pushToken) return json({ error: 'pushToken' }, 400)

  const environment = readEnvironment(body.environment)
  if (environment instanceof Response) return environment

  const proven = await provenFingerprint(body)
  if (proven instanceof Response) return proven

  // The same key arriving on a different push token: a reinstall, or Apple
  // reissuing one. There is one row per key per account and it is a unique
  // index, so without moving the key off the old row the insert below violates
  // it — and this is the call push depends on, so it would fail on every
  // launch, permanently, on the one path that has to keep working.
  //
  // The old row stays in the device list, holding a token nothing answers on,
  // and loses the key because the key is not there any more. Its standing goes
  // with the key: a ceremony verified THIS key, and whoever is registering has
  // just proved they still hold it, which is more than the row being replaced
  // can say.
  let inherited: string | null = null
  if (proven) {
    const previous = await env.DB.prepare(
      `SELECT id, state FROM devices
        WHERE account_id = ?1 AND key_a_fingerprint = ?2
          AND NOT (platform = ?3 AND push_token = ?4)`,
    )
      .bind(account, proven, body.platform, body.pushToken)
      .first<{ id: string; state: string }>()
    if (previous) {
      inherited = previous.state
      await env.DB.prepare(`UPDATE devices SET key_a_fingerprint = NULL WHERE id = ?`)
        .bind(previous.id)
        .run()
    }
  }

  // `version`, `environment`, the push-to-start token and now the fingerprint
  // are all optional and all COALESCEd: an App Store build from before a column
  // existed still registers, and — this is the part that bit `version` first —
  // re-registering from that older build must not erase what a newer one
  // reported. `label` is assigned rather than coalesced because every build has
  // always sent it, so an absent one is a rename to the default and not an old
  // client.
  //
  // Every column added by a migration has to be named HERE as well. The upsert
  // lists what it updates, so a new column that is not in this list is written
  // on insert and never again: an updated app would re-register with a 200 and
  // stay legacy forever — fingerprint NULL, invisible to every ceremony, with
  // nothing on any screen saying why. That is what the regression test in
  // `test/relay.test.ts` is for.
  //
  // `state` is a CASE rather than an assignment because three cases are three
  // different answers. A client that sent no key changes nothing. A row that
  // predates fingerprints keeps what it had, because it was created by the flow
  // that WAS the trust model before this — demoting every installed device
  // would leave a fleet where nothing is verified and nothing can promote
  // anything, since promotion needs a device that already is. And a DIFFERENT
  // key on the same device goes back to pending: possession is proven, a
  // ceremony is not, and inheriting `verified` would let a key nobody enrolled
  // take the standing of the one it replaced.
  await env.DB.prepare(
    `INSERT INTO devices
       (id, account_id, platform, push_token, label, version, environment,
        live_activity_start_token, key_a_fingerprint, state, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (platform, push_token)
     DO UPDATE SET account_id = excluded.account_id,
                   label = excluded.label,
                   version = COALESCE(excluded.version, devices.version),
                   environment = COALESCE(excluded.environment, devices.environment),
                   live_activity_start_token = COALESCE(
                     excluded.live_activity_start_token, devices.live_activity_start_token),
                   key_a_fingerprint = COALESCE(
                     excluded.key_a_fingerprint, devices.key_a_fingerprint),
                   state = CASE
                             WHEN excluded.key_a_fingerprint IS NULL THEN devices.state
                             WHEN devices.key_a_fingerprint IS NULL THEN devices.state
                             WHEN devices.key_a_fingerprint = excluded.key_a_fingerprint
                               THEN devices.state
                             ELSE 'pending'
                           END,
                   updated_at = excluded.updated_at`,
  )
    .bind(
      crypto.randomUUID(),
      account,
      body.platform,
      body.pushToken,
      body.label ?? 'Device',
      typeof body.version === 'string' ? body.version.slice(0, 64) : null,
      environment,
      typeof body.liveActivityStartToken === 'string' && body.liveActivityStartToken
        ? body.liveActivityStartToken
        : null,
      proven,
      // A new row is `pending`: it proves possession of a key, which is not a
      // ceremony having enrolled it. `verified` for a registration with no key
      // at all, matching the column's default — such a row carries no
      // fingerprint, so it matches no lookup and the state grants it nothing.
      inherited ?? (proven ? 'pending' : 'verified'),
      Date.now(),
    )
    .run()

  await record(env.METRICS, env.ANALYTICS_SALT, 'device_registered', account, {
    platform: body.platform,
  })
  return json({ ok: true })
}

/// What an app sends to register. Everything but the platform and the token is
/// optional, and stays optional: a build shipped before a field existed sends
/// none of it and gets exactly the behavior it always got.
interface Registration {
  platform: string
  pushToken: string
  label?: string
  version?: string
  environment?: unknown
  /// The device's own install identifier, which is what it signs. Not stored:
  /// the row's id is generated here.
  deviceId?: unknown
  /// Key A's public half, as an `ssh-ed25519 AAAA…` line.
  keyA?: unknown
  /// What the device believes its own fingerprint is, checked rather than used.
  fingerprint?: unknown
  /// A raw ed25519 signature over `deviceId`, base64.
  signature?: unknown
  liveActivityStartToken?: unknown
}

/// The fingerprint this registration PROVED it holds, or the response to send
/// instead.
///
/// Registration used to record whatever fingerprint a session-holder sent, and
/// the account gate would then be checking membership of a registry rather than
/// that the device in front of you holds the key it is showing — a fingerprint
/// is public, off a screen or out of a QR code, so anyone who had seen one could
/// register it. A signature closes that: the fingerprint stored is derived FROM
/// the key that verified, so what is recorded and what was proved cannot
/// disagree.
///
/// `null`, not an error, when the request carries no key material at all. That
/// is every app already in the App Store, and refusing them would take push
/// down for everyone installed on the day this deploys — while buying nothing,
/// because a row with no fingerprint matches no lookup and so claims nothing
/// about any key. What must never happen is a fingerprint recorded WITHOUT a
/// signature, and that is what this refuses.
///
/// The signature is over the device id the request names rather than over the
/// row's id, because the row's id is generated here and a new device has never
/// been told it — there would be nothing for it to sign. What the signature
/// establishes is possession, and possession is what the gate needs.
///
/// A `fingerprint` in the body is compared against the derived one rather than
/// trusted. It is the string the device puts on its own screen for a person to
/// compare at the confirmation, so a client computing it differently from this
/// relay would leave two screens showing two strings that can never match — and
/// that is worth failing at registration rather than discovering mid-ceremony.
async function provenFingerprint(body: Registration): Promise<string | null | Response> {
  const offered = body.keyA !== undefined || body.signature !== undefined
  if (!offered && body.fingerprint === undefined) return null

  if (
    typeof body.keyA !== 'string' ||
    typeof body.signature !== 'string' ||
    typeof body.deviceId !== 'string' ||
    !body.keyA ||
    !body.signature ||
    !body.deviceId
  ) {
    return json({ error: 'signature' }, 400)
  }

  const key = parseEd25519(body.keyA)
  if (!key) return json({ error: 'keyA' }, 400)
  if (!(await verifyEd25519(key, body.signature, body.deviceId))) {
    return json({ error: 'signature' }, 400)
  }

  const fingerprint = await fingerprintOf(key)
  if (typeof body.fingerprint === 'string' && body.fingerprint !== fingerprint) {
    return json({ error: 'fingerprint' }, 400)
  }
  return fingerprint
}

/// Does this account have a device holding this key?
///
/// The one question the relay answers about a key, and it answers no other. The
/// account is bound IN the query rather than compared after it: a lookup by key
/// alone would return some device and leave the caller checking account ids,
/// which breaks the moment two accounts have registered the same public key —
/// and it would make this route a key-enumeration oracle for everyone else's
/// devices. Scoped, the answer is "yes, on your account" or "no", and never
/// whose a key is otherwise.
///
/// THERE IS NO STATE PREDICATE HERE, and adding one as a hardening would break
/// every onboarding there is. The gate is account membership, and it is
/// satisfied the moment a device holding that account's session proved
/// possession of the key. Whether a ceremony has COMPLETED is a different
/// question — it governs how the device list draws the row and whether a later
/// grant may name it, not whether this ceremony may proceed. The two cannot be
/// one predicate: a row is `pending` until a ceremony completes, a ceremony
/// cannot complete until the trusted device has looked the key up, and a lookup
/// that required `verified` would be waiting on its own result. So the state is
/// reported rather than filtered, and the caller can say "this device has not
/// completed a ceremony" without asking again.
///
/// This gate is not relay-proof and is not claimed to be: a compromised relay
/// can answer yes to anything. What it cannot do is produce a fingerprint at the
/// confirmation, which is the gate that remains.
async function lookupDevice(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const body = await request.json<{ fingerprint?: unknown }>()
  if (typeof body.fingerprint !== 'string' || !body.fingerprint) {
    return json({ error: 'fingerprint' }, 400)
  }

  const device = await env.DB.prepare(
    `SELECT id, label, state FROM devices
     WHERE key_a_fingerprint = ?1 AND account_id = ?2`,
  )
    .bind(body.fingerprint, account)
    .first<{ id: string; label: string; state: string }>()

  // Nothing but `false` on a miss. Not the state, not a reason, not a count:
  // each of those is a way of asking the relay about a key that is not yours,
  // and a key on another account has to be indistinguishable from a key on no
  // account at all.
  if (!device) return json({ found: false })
  return json({ found: true, label: device.label, state: device.state })
}

/// A ceremony finished: the device holding this key is one of ours.
///
/// Called by the TRUSTED device once it has actually enrolled the new one, which
/// is why this is the promotion and registration is not. The new device is the
/// only party that can prove possession, and a trusted device is the only party
/// that can say a ceremony happened; neither half is enough alone, and that is
/// the whole point of there being two states.
///
/// Only from `pending`, and only within this account. `ok` reports whether a row
/// actually moved — answering true regardless would make "a ceremony completed
/// just now" indistinguishable from "this key was already here", which is the
/// one thing the caller is asking.
async function verifyDevice(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const body = await request.json<{ fingerprint?: unknown }>()
  if (typeof body.fingerprint !== 'string' || !body.fingerprint) {
    return json({ error: 'fingerprint' }, 400)
  }

  const result = await env.DB.prepare(
    `UPDATE devices SET state = 'verified'
     WHERE key_a_fingerprint = ?1 AND account_id = ?2 AND state = 'pending'
       AND updated_at > ?3`,
  )
    .bind(body.fingerprint, account, Date.now() - PENDING_LIFETIME_MS)
    .run()

  const promoted = (result.meta?.changes ?? 0) > 0
  if (promoted) await record(env.METRICS, env.ANALYTICS_SALT, 'device_verified', account)
  return json({ ok: promoted })
}

/// How long a registration may wait for the ceremony that completes it.
///
/// A pending row is created moments before a ceremony and promoted at the end of
/// one, so a day is already generous. Past it, a promotion would be a
/// registration from some earlier time being completed by a later session, which
/// is not what anyone in the room is doing. The row is not deleted: it is a
/// device somebody registered, it shows in the list as unverified, and the app
/// re-registering renews it.
const PENDING_LIFETIME_MS = 24 * 60 * 60 * 1000

/// Remember how to reach the Live Activity currently running for a terminal.
///
/// Separate from `/v1/devices` because the two tokens have nothing in common
/// but the word: the push-to-start token belongs to the app install and is
/// known at registration, while an update token exists only once an activity is
/// already running and dies with it. The app has to report it after the fact,
/// and there is no moment at which both are known together.
///
/// `updateToken: null` is how the app says the activity is over, and `dismissed`
/// says whether the PERSON ended it — see the sentinel handling below.
async function registerActivity(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account

  const body = await request.json<{
    terminal?: unknown
    updateToken?: unknown
    environment?: unknown
    dismissed?: unknown
  }>()
  // The row is keyed on the terminal. Without one, every agent on the account
  // would collapse onto a single card that contradicts all of them.
  if (typeof body.terminal !== 'string' || !body.terminal) return json({ error: 'terminal' }, 400)

  const environment = readEnvironment(body.environment)
  if (environment instanceof Response) return environment

  if (body.updateToken === null) {
    // A card the PERSON swiped away is remembered rather than forgotten.
    //
    // Deleting the row was how a dismissed card came back: the daemon pushes a
    // working card about every ten seconds, the next one found nothing running,
    // and it started the card again — for the rest of the run. Swiping something
    // away and having it return within ten seconds is not a card that is hard to
    // dismiss, it is a card that cannot be.
    //
    // The row is kept with the sentinel — there is no card to address any more —
    // and `dismissed_at` set, which `pushActivity` reads as "no card, and the
    // person meant it". A `blocked` push still raises a fresh one, because that
    // is news they have not seen; `done` deletes the row with the run.
    //
    // Only when the app says the person did it. An activity that merely ENDED —
    // the relay's own `end` push, iOS retiring a stale card — deletes the row as
    // before, because there is no refusal to remember there.
    if (body.dismissed === true) {
      await env.DB.prepare(
        `UPDATE live_activities
         SET update_token = ?, dismissed_at = ?, updated_at = ?
         WHERE account_id = ? AND terminal = ?`,
      )
        .bind(TOKEN_UNKNOWN, Date.now(), Date.now(), account, body.terminal)
        .run()
      return json({ ok: true })
    }
    await env.DB.prepare(`DELETE FROM live_activities WHERE account_id = ? AND terminal = ?`)
      .bind(account, body.terminal)
      .run()
    // `ok` unconditionally, unlike `revokeOwned`: there may legitimately be no
    // row, because ending an activity from `/v1/notify` already deleted it, and
    // the app reporting the same truth a moment later has not failed at
    // anything.
    return json({ ok: true })
  }
  if (typeof body.updateToken !== 'string' || !body.updateToken) {
    return json({ error: 'updateToken' }, 400)
  }

  // The token is assigned rather than coalesced: APNs issues a new one per
  // activity and the previous one is already dead, so keeping it would leave
  // the relay pushing at an address nothing is listening to.
  //
  // `blind_status` and `dismissed_at` are cleared with it. Both describe a card
  // the relay could not reach, and a real update token is the end of that: there
  // is a card, it is up, and it can be moved in place from here on.
  await env.DB.prepare(
    `INSERT INTO live_activities (id, account_id, terminal, update_token, environment, updated_at)
     VALUES (?, ?, ?, ?, ?, ?)
     ON CONFLICT (account_id, terminal)
     DO UPDATE SET update_token = excluded.update_token,
                   environment = COALESCE(excluded.environment, live_activities.environment),
                   blind_status = NULL,
                   dismissed_at = NULL,
                   updated_at = excluded.updated_at`,
  )
    .bind(crypto.randomUUID(), account, body.terminal, body.updateToken, environment, Date.now())
    .run()

  return json({ ok: true })
}

/// The APNs environment a request named, or the response to send instead.
///
/// Absent is NULL, which reads as production, because that is what every client
/// built before this field existed is. Anything else is a 400 rather than a
/// fall-through to production: a fall-through is precisely the bug the field
/// was added to fix, and one that a typo could reinstate without a trace.
function readEnvironment(value: unknown): Environment | null | Response {
  if (value === undefined || value === null) return null
  if (isEnvironment(value)) return value
  return json({ error: 'environment' }, 400)
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

  // `state`, because a row that is not verified has to be visible as such. A
  // device half way through a ceremony, or one whose app has not been updated
  // since fingerprints existed, is invisible to the account lookup — and the
  // design's answer for both is that the list says so, pointing at the device,
  // rather than a ceremony failing later with nothing on screen explaining it.
  const devices = await env.DB.prepare(
    `SELECT id, platform, label, version, state, updated_at FROM devices
     WHERE account_id = ? ORDER BY updated_at DESC`,
  )
    .bind(account)
    .all<{
      id: string
      platform: string
      label: string
      version: string | null
      state: string
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
      state: d.state,
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
/// The body is deliberately thin, and thin means one composed line at a time:
/// `subtitle` is the agent's question while it is blocked and its composed
/// signal rung while it works, redacted and cut to a sidebar's width by the
/// runner before it is sent. Never the transcript, never a command line, never
/// raw output.
///
/// It arrives repeatedly, which is newer than it looks. A `working` notice
/// moves the live card for the whole length of a run — every ten seconds at
/// most — where a run used to send about two notices in total. So the relay
/// sees a slow drip of one agent's headline. That is still not something it
/// holds: this route persists `version` and nothing else off the body,
/// `live_activities` keeps delivery metadata, and the top-level catch refuses
/// to log a body at all. The rule stands — the relay has no business holding a
/// conversation's contents in transit — but it is a rule about a stream now,
/// not about two lines.
///
/// `status` and `label` are the newest fields and, like every field added here,
/// optional forever: a daemon built before them sends neither and gets exactly
/// the behavior it always got, which is the alert push and nothing else.
interface Notification {
  title: string
  subtitle?: string
  terminal?: string
  version?: string
  status?: string
  label?: string
  /// Whether the turn behind a `done` ended badly, for the notification service
  /// extension's mark. Forwarded and never acted on here — see `push.ts`.
  failed?: boolean
  /// When the turn began, in Unix milliseconds, for the card's own clock.
  ///
  /// A timestamp is not content: it says WHEN something started and nothing
  /// about what it is, so it does not widen what this relay holds in transit —
  /// which is the rule the thin body above exists to keep.
  ///
  /// Only a `start` can carry it onward. Attributes are an activity's identity
  /// and APNs rejects a push that repeats them, so this reaches the phone on
  /// the card's first push or not at all.
  startedAt?: number
}

interface Device {
  platform: string
  push_token: string
  environment: string | null
  live_activity_start_token: string | null
}

async function notify(request: Request, env: Env): Promise<Response> {
  const header = request.headers.get('authorization') ?? ''
  const token = header.startsWith('Bearer ') ? header.slice(7) : ''
  if (!token) return json({ error: 'unauthorized' }, 401)

  // Expiry checked in the WHERE clause, not after: a token that has run out is
  // a token that does not exist. NULL means a pairing issued before expiries
  // existed, which keeps working — logging those machines out to introduce a
  // policy would break a feature people had just set up.
  //
  // `label` comes along because it is the machine's name, which is one of the
  // three things a Live Activity's card says.
  const daemon = await env.DB.prepare(
    `SELECT id, account_id, label FROM daemons
     WHERE token_hash = ? AND (expires_at IS NULL OR expires_at > ?)`,
  )
    .bind(await sha256(token), Date.now())
    .first<{ id: string; account_id: string; label: string }>()
  if (!daemon) return json({ error: 'unauthorized' }, 401)

  const body = await request.json<Notification>()
  if (!body.title) return json({ error: 'title' }, 400)

  // A misconfigured deployment, said out loud rather than delivered as silence.
  //
  // `apns-topic` must equal the receiving app's bundle id and each channel has
  // its own, so a relay deployed as one channel holding another's topic has
  // every push rejected by APNs — with `sendApns` returning false and the
  // daemon told only that a notification "failed". This is the one secret whose
  // wrongness is invisible, and it is invisible on the path people are least
  // likely to be watching.
  //
  // 500, not 400: nothing is wrong with what the machine asked for. Refused
  // before any device is read, because delivering to none of them and calling
  // it a delivery is the failure being prevented.
  const misconfigured = topicMismatch(env)
  if (misconfigured) {
    console.error(`relay misconfigured: ${misconfigured}`)
    return json({ error: 'relay misconfigured', detail: misconfigured }, 500)
  }

  const devices = await env.DB.prepare(
    `SELECT platform, push_token, environment, live_activity_start_token
     FROM devices WHERE account_id = ?`,
  )
    .bind(daemon.account_id)
    .all<Device>()

  // A working state moves the card and nothing else — it may even create the
  // card, but it never sends an alert push. The rule that a working agent must
  // not buzz is unchanged; only the card is new — and an agent being busy is the
  // normal case, so a banner for it is a banner people switch off, taking the
  // blocked and done ones with it.
  //
  // `delivered` therefore stays 0 for a working notify, which the daemon treats
  // as a success: the 200 is what it checks, not the count.
  let delivered = 0
  if (body.status !== 'working') {
    for (const device of devices.results ?? []) {
      const ok = await sendPush(
        env,
        device.platform,
        device.push_token,
        {
          title: body.title,
          subtitle: body.subtitle ?? '',
          terminal: body.terminal ?? '',
          status: body.status,
          label: body.label,
          failed: body.failed,
        },
        device.environment,
      )
      if (ok) delivered += 1
      await record(
        env.METRICS,
        env.ANALYTICS_SALT,
        ok ? 'notification_sent' : 'notification_failed',
        daemon.account_id,
        { platform: device.platform, ok },
      )
    }
  }

  // The lock screen comes second, and never at the alert's expense.
  //
  // After the loop above so that a Live Activity push cannot delay or displace
  // the alert, and swallowed so that a dead activity token cannot turn a
  // notification that WAS delivered into a 500 — the daemon would retry it, and
  // the user would be interrupted twice for one event.
  try {
    await pushActivity(env, daemon, body, devices.results ?? [])
  } catch (error) {
    console.error('live activity push failed', error)
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

/// The `update_token` of a card the relay started while the app was not running.
///
/// A row HAS to exist the moment a card is push-started, or the next push finds
/// none and starts another card. The daemon sends `working` about every ten
/// seconds for the length of a run, so a half-hour run would leave on the order
/// of a hundred and eighty cards on the lock screen — none of which the relay
/// holds an update token for, and none of which it can therefore ever end. This
/// row is what `UNIQUE (account_id, terminal)` refuses the second start against,
/// which is the invariant the comment on that constraint already claims.
///
/// The empty string rather than NULL because `live_activities.update_token` is
/// declared NOT NULL, the migrations in this service are additive only — see the
/// header of 0003 for why — and SQLite cannot loosen a column in place.
///
/// It is NOT a token and must never reach APNs: an activity push addressed to
/// the empty string addresses no activity. Every read of `update_token` has to
/// ask whether it is this first, and the one place that writes a real one,
/// `/v1/devices/activity`, already 400s an empty `updateToken` — so the app
/// cannot report this value by accident.
const TOKEN_UNKNOWN = ''

/// How long a swipe keeps this terminal off the lock screen.
///
/// A dismissal is remembered on the row, and `done` deletes the row with the
/// run — but a run can end without a `done` ever arriving: a daemon killed
/// mid-turn, a machine that slept. A row kept forever on that path would refuse
/// this terminal a card for every run that followed, silently and permanently,
/// which is a worse failure than the undismissable card the memory was added to
/// fix.
///
/// An hour, which is `STALE_AFTER_S` in `services/relay/src/push.ts` and the
/// same number for the same reason: after that long with no news the relay does
/// not know anything about this terminal, including whether the card the person
/// swiped away was about the run still running now.
const DISMISSAL_MEMORY_MS = 60 * 60 * 1000

/// Put what just happened on the lock screen, if the daemon said enough for it
/// to mean anything.
///
/// Returns without doing a thing unless the daemon named a status this relay
/// understands. An unrecognized one is IGNORED rather than rejected: the daemon
/// ships separately from the relay and will eventually send a status invented
/// after this code was written, and a 400 there would cost the user the alert —
/// the one part of this route that is actually promised.
async function pushActivity(
  env: Env,
  daemon: { account_id: string; label: string },
  body: Notification,
  devices: Device[],
): Promise<void> {
  const status = body.status
  if (status !== 'blocked' && status !== 'done' && status !== 'working') return

  // No terminal, no activity. `live_activities` is keyed on it, and a row under
  // the empty string is every agent on the account sharing one card that
  // contradicts all of them — worse than no card at all.
  const terminal = body.terminal ?? ''
  if (!terminal) return

  const state: ActivityState = {
    status,
    // The daemon's own words when it has any. When it has none, a blocked card
    // still needs to say what it is waiting for — "Needs You" alone does not —
    // but a finished one does not, because the card already reads "Finished"
    // above this line and repeating it is the sort of thing you cannot unsee.
    detail: body.subtitle || (status === 'blocked' ? 'Waiting for your answer' : ''),
  }
  // What separates the tiers is the alert, not whether a push goes out at all.
  // An activity push carrying an alert dictionary is PRESENTED — lock screen
  // banner, Apple Watch haptic — and one without it changes the card in place
  // and says nothing. `blocked` has earned that interruption and `done` closes
  // it out; `working` never earns it, at any point in the card's life. A banner
  // every ten seconds for an agent that is merely busy is the notification
  // people switch the app off over, which then costs them the one push this
  // whole product exists to deliver.
  const alert = status === 'working' ? undefined : { title: body.title, body: body.subtitle ?? '' }

  const running = await env.DB.prepare(
    `SELECT update_token, environment, blind_status, dismissed_at FROM live_activities
     WHERE account_id = ? AND terminal = ?`,
  )
    .bind(daemon.account_id, terminal)
    .first<{
      update_token: string
      environment: string | null
      blind_status: string | null
      dismissed_at: number | null
    }>()

  if (running) {
    // There is a card, so every status is a change to it in place — including
    // `working`, which is the ordinary case and the reason the card moves at
    // all.
    //
    // Null when the relay started this card itself and the app has not run
    // since. The row proves a card exists; only the app can learn the update
    // token that addresses it, so until then there is genuinely nowhere to send
    // anything. See `TOKEN_UNKNOWN`.
    const address = running.update_token === TOKEN_UNKNOWN ? null : running.update_token

    // `done` is the last status and the row goes with it either way: the update
    // token dies with the activity it was issued for, and a row left behind
    // would refuse this terminal a card for every run that follows. An
    // unaddressed card is left to the `stale-date` its start carried, which is
    // the bounded hole push-to-start has always had — better than a permanent
    // one.
    if (status === 'done') {
      if (address) {
        await deliverActivity(env, daemon.account_id, address, running.environment, {
          event: 'end',
          state,
          alert,
        })
      }
      await env.DB.prepare(`DELETE FROM live_activities WHERE account_id = ? AND terminal = ?`)
        .bind(daemon.account_id, terminal)
        .run()
      return
    }

    if (!address) {
      // The card cannot be moved, so the only question left is whether the
      // person is looking at something WRONG.
      //
      // `blocked` after a blind `working` start is exactly that: the lock screen
      // reads "Working" while the banner beside it says the agent needs you, and
      // it reads that way until the run ends or the hour-long stale date passes.
      // That is the product's primary scenario stating the opposite of the
      // truth, so a fresh card is started and the old one is left to expire —
      // briefly two cards, one of them out of date, rather than one card that is
      // wrong. Also the path for a card the person DISMISSED, where there is no
      // duplicate at all: a new question is news they have not answered.
      //
      // Exactly once. `blind_status` remembers the tier this row's card is
      // showing, so a second `blocked` push finds its own status here and starts
      // nothing — without it, every push while unaddressable would stack another
      // card, which is the failure the row itself exists to prevent.
      //
      // `working` never takes this path. It is the silent tier, it has no news
      // to correct, and re-raising a card the person swiped away is the thing
      // that made a dismissed card come back within ten seconds.
      if (status === 'blocked' && running.blind_status !== 'blocked') {
        await startCard(env, daemon, body, devices, terminal, state, alert)
        return
      }

      // A dismissal this row has held long enough. See `DISMISSAL_MEMORY_MS`:
      // the row is deleted rather than merely ignored, because a run that is
      // still going an hour later is a run whose card should exist, and leaving
      // the row would refuse it one for as long as the daemon keeps pushing.
      if (
        running.dismissed_at !== null &&
        Date.now() - running.dismissed_at >= DISMISSAL_MEMORY_MS
      ) {
        await env.DB.prepare(`DELETE FROM live_activities WHERE account_id = ? AND terminal = ?`)
          .bind(daemon.account_id, terminal)
          .run()
        await startCard(env, daemon, body, devices, terminal, state, alert)
      }
      return
    }

    await deliverActivity(env, daemon.account_id, address, running.environment, {
      event: 'update',
      state,
      alert,
    })
    return
  }

  // Nothing to end, and nothing to start either: a push-to-start announcing
  // that something already finished leaves a card on the lock screen that the
  // relay can never take back, because the app never gets an update token for
  // an activity it did not know was coming.
  if (status === 'done') return

  await startCard(env, daemon, body, devices, terminal, state, alert)
}

/// Raise a card from the outside, and remember that it is up.
///
/// This is the reason the push-to-start token is stored at all: the agent starts
/// working, or blocks, while the phone is in a pocket, and there is nothing
/// awake on the device to start a card.
///
/// `working` starts a card here, and that is the feature: the card follows a
/// whole run — busy, then blocked, then finished — rather than appearing only
/// once something has already gone wrong, and the Dynamic Island is empty for
/// the entire stretch there is anything to watch otherwise.
///
/// It starts SILENTLY for `working`. `alert` is undefined for that status, so
/// the push creates the card without presenting anything; the interruption stays
/// the exclusive property of `blocked`. A start that carried an alert would buzz
/// the wrist every time any agent picked up work, which is the failure every
/// comment in `pushActivity` exists to prevent.
async function startCard(
  env: Env,
  daemon: { account_id: string; label: string },
  body: Notification,
  devices: Device[],
  terminal: string,
  state: ActivityState,
  alert: { title: string; body: string } | undefined,
): Promise<void> {
  const starters = devices.filter(
    (device): device is Device & { live_activity_start_token: string } =>
      device.platform === 'apns' && !!device.live_activity_start_token,
  )
  // Nothing on the account can raise a card, so nothing was started and there is
  // nothing to remember. Claiming the terminal here would refuse a card for the
  // rest of the run to a phone that registers its push-to-start token a minute
  // from now.
  if (starters.length === 0) return

  // Remember the card BEFORE the first push goes out, and never after.
  //
  // Written first because a push that throws is ambiguous — APNs may well have
  // created the card — and the two ways of being wrong are not comparable. Claim
  // first and a failed start costs this run its card, silently. Claim last and a
  // start that really happened is forgotten, and the next `working` ten seconds
  // later starts another, forever. `TOKEN_UNKNOWN` says the rest.
  //
  // The conflict arm updates only a row that is still unaddressable, which is
  // what makes it safe. The app can file the real update token between the
  // caller's SELECT and this INSERT — that is exactly what happens when the
  // phone comes to the foreground — and overwriting it with the sentinel would
  // throw away the only address the card has. The `WHERE` is what refuses that,
  // while still letting an escalation record the tier its new card is showing
  // and clear a dismissal it has just superseded.
  //
  // `environment` stays NULL because nothing knows it yet: the start goes to
  // every phone on the account, and whichever one's app runs next reports its
  // own environment alongside the token it files.
  await env.DB.prepare(
    `INSERT INTO live_activities
       (id, account_id, terminal, update_token, environment, blind_status, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (account_id, terminal)
     DO UPDATE SET blind_status = excluded.blind_status,
                   dismissed_at = NULL,
                   updated_at = excluded.updated_at
     WHERE live_activities.update_token = ?`,
  )
    .bind(
      crypto.randomUUID(),
      daemon.account_id,
      terminal,
      TOKEN_UNKNOWN,
      null,
      state.status,
      Date.now(),
      TOKEN_UNKNOWN,
    )
    .run()

  for (const device of starters) {
    await deliverActivity(
      env,
      daemon.account_id,
      device.live_activity_start_token,
      device.environment,
      {
        event: 'start',
        state,
        alert,
        attributes: {
          terminal,
          // The agent's name if the daemon sent one, and the notification's
          // title if it did not. A blank line on the card would be worse than
          // repeating a line the person has already read.
          label: body.label || body.title,
          machine: daemon.label,
          // The turn's clock, and the only push that can carry it: attributes
          // are fixed for an activity's life, so a card started without this
          // shows no elapsed time for as long as it exists.
          //
          // Type-checked rather than trusted, the same way `version` is below.
          // The body is whatever a machine posted, and the app decodes this
          // field as a number — a string reads back as nil there and costs the
          // timer silently, so a `null` or a `"1755..."` is dropped here where
          // it is still visible rather than on a lock screen where it is not.
          startedAt: typeof body.startedAt === 'number' ? body.startedAt : undefined,
        },
      },
    )
  }
}

/// One activity push, counted.
///
/// A separate event name from the alert's on purpose: these fail for reasons
/// the alert does not — an update token that outlived its activity, a payload
/// the app cannot decode — and folding them into `notification_failed` would
/// make the delivery rate that actually matters look worse than it is.
async function deliverActivity(
  env: Env,
  account: string,
  token: string,
  environment: string | null,
  activity: Activity,
): Promise<void> {
  const ok = await sendLiveActivity(env, token, activity, environment)
  await record(
    env.METRICS,
    env.ANALYTICS_SALT,
    ok ? 'activity_sent' : 'activity_failed',
    account,
    { platform: 'apns', ok },
  )
}

// MARK: - Helpers

/// The signed-in account, or the response to send instead.
async function requireAccount(request: Request, env: Env): Promise<string | Response> {
  const header = request.headers.get('authorization') ?? ''
  if (!header.startsWith('Bearer ')) return json({ error: 'unauthorized' }, 401)

  const session = await verifySession(header.slice(7), env)
  if (!session) return json({ error: 'unauthorized' }, 401)

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
