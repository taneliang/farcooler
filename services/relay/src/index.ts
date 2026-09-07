/// The relay's routes.
///
/// Three groups, and the asymmetry between them is the whole security model.
/// Signing in exchanges a WorkOS code for a session. Registering a device and
/// pairing a machine are done BY A SIGNED-IN PERSON. The `/v1/notify` pair is
/// done by a machine holding a token that person issued — so a machine can only
/// ever notify the account that paired it, and only ever take down that
/// account's cards. Neither of the two names a destination; see `/v1/notify`.
///
/// The apps hold a WorkOS client id, which is public by design, and never an API
/// key. The code exchange happens here because that is the one step that needs
/// the secret, and a secret in a repo that is about to be open source — or in an
/// app bundle anyone can unzip — is not a secret.

import { record, type Metrics } from './analytics'
import { fingerprintOf, parseEd25519, verifyEd25519 } from './keys'
import { verifySession } from './workos'
import {
  ACTIVITY_VERSION,
  isEnvironment,
  sendLiveActivity,
  sendPush,
  topicMismatch,
  type Activity,
  type ActivityRow,
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
        case '/v1/notify/retire':
          return await retireActivities(request, env)
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

  // `version`, `environment`, the push-to-start token, the fingerprint and now
  // the done preference are all optional and all COALESCEd: an App Store build
  // from before a column existed still registers, and — this is the part that
  // bit `version` first — re-registering from that older build must not erase
  // what a newer one reported. For `notify_on_done` the COALESCE is also what
  // keeps an absent field reading as "notify" rather than "off", which is the
  // difference between a deploy that changes nothing for old builds and one
  // that silences them. `label` is assigned rather than coalesced because every build has
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
        live_activity_start_token, key_a_fingerprint, notify_on_done, state, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (platform, push_token)
     DO UPDATE SET account_id = excluded.account_id,
                   label = excluded.label,
                   version = COALESCE(excluded.version, devices.version),
                   environment = COALESCE(excluded.environment, devices.environment),
                   live_activity_start_token = COALESCE(
                     excluded.live_activity_start_token, devices.live_activity_start_token),
                   key_a_fingerprint = COALESCE(
                     excluded.key_a_fingerprint, devices.key_a_fingerprint),
                   notify_on_done = COALESCE(
                     excluded.notify_on_done, devices.notify_on_done),
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
      // Only a real boolean says anything. Anything else — absent, null, a
      // string an old or confused client sent — is NULL, which the fan-out
      // reads as "notify".
      typeof body.notifyOnDone === 'boolean' ? (body.notifyOnDone ? 1 : 0) : null,
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
  /// Whether this device wants to be told a turn ended — the "When an agent
  /// finishes or fails" toggle, sent up so it reaches pushes as well as the
  /// banners the app draws itself.
  ///
  /// Absent means notify. A build that predates the field sends nothing and
  /// keeps the behavior it has; see the COALESCE below and migration 0007.
  notifyOnDone?: unknown
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

/// Remember how to reach the Live Activity this install currently has running.
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
  // `terminal` is read no more. The row was keyed on it and is keyed on the
  // install now — see `install_cards` — so the field this route once refused a
  // request for is a field it has nothing to do with.
  //
  // Still ACCEPTED, and still sent by the app, because the two ends of this
  // route are one deployment per channel but not one atomic one: an app talking
  // to a relay that predates the rekey must keep sending it, and this relay must
  // keep taking it from an app that does. Ignoring a field is the only
  // compatible way to retire one.

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
    // is news they have not seen; `done` for the card's leader deletes the row
    // with the run.
    //
    // What the swipe now covers is the whole card rather than one agent, and
    // that is the honest reading of the gesture: there is ONE card, it leads
    // with one agent and counts the rest, and a person swiping it away is
    // refusing that card and not making a statement about a terminal they were
    // never shown an id for. The alert banners are untouched either way — they
    // are a separate push and a separate decision.
    //
    // Only when the app says the person did it. An activity that merely ENDED —
    // the relay's own `end` push, iOS retiring a stale card — deletes the row as
    // before, because there is no refusal to remember there.
    if (body.dismissed === true) {
      await env.DB.prepare(
        `UPDATE install_cards
         SET update_token = ?, dismissed_at = ?, updated_at = ?
         WHERE account_id = ?`,
      )
        .bind(TOKEN_UNKNOWN, Date.now(), Date.now(), account)
        .run()
      return json({ ok: true })
    }
    await env.DB.prepare(`DELETE FROM install_cards WHERE account_id = ?`).bind(account).run()
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
  // `dismissed_at` is cleared with it. It describes a card the relay could not
  // reach, and a real update token is the end of that: there is a card, it is
  // up, and it can be moved in place from here on.
  //
  // The LEADER is deliberately not touched. It used to be — `blind_status` was
  // set to NULL here — because that column only ever meant "what the card the
  // relay started blind is showing", and an addressable card had no use for it.
  // `leader_terminal` and `leader_status` mean something the relay still needs
  // once it can reach the card: which agent this card is about, so the next
  // push can decide whether it outranks that one. Clearing them here would make
  // every card forget its leader the moment the app came to the foreground.
  //
  // They are NULL only in the insert arm, which is the app filing a token for a
  // card the relay holds no row for — an `end` that raced the app's report, a
  // card left over from an older build. A NULL leader is adopted by the next
  // push; see `pushActivity`.
  await env.DB.prepare(
    `INSERT INTO install_cards
       (id, account_id, update_token, environment, leader_terminal, leader_status, updated_at)
     VALUES (?, ?, ?, ?, NULL, NULL, ?)
     ON CONFLICT (account_id)
     DO UPDATE SET update_token = excluded.update_token,
                   environment = COALESCE(excluded.environment, install_cards.environment),
                   dismissed_at = NULL,
                   updated_at = excluded.updated_at`,
  )
    .bind(crypto.randomUUID(), account, body.updateToken, environment, Date.now())
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
/// `install_cards` keeps delivery metadata, and the top-level catch refuses
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
  /// This agent's diff against its base, and the commits inside its trace's
  /// window. **The first numbers this body has ever carried.**
  ///
  /// The card grew a row per agent — `auth-refactor  force-push?  +142 −37
  /// 4 commits` — and the relay is the only place that can compose one. A fleet
  /// spans several runners, each with its own daemon pushing independently, so
  /// no daemon sees the whole fleet and none can total it; this worker sees
  /// every runner's notices for one account. That is why these arrive here and
  /// why the relay keeps them, which is a change to the rule stated above and
  /// is stated as one: see `rememberAgent`, and the columns migration 0008 adds.
  ///
  /// A count is not content. It says how MUCH happened and nothing about what,
  /// which is strictly less than the composed line `subtitle` already carries
  /// across this same wire.
  ///
  /// **Absent is not zero, and this is the field that has to hold that line.** A
  /// worktree nobody has probed and one with no base to compare against have
  /// both said nothing; the daemon omits the key rather than sending a
  /// confident zero, the column stays NULL, and the row draws no numbers.
  /// `undefined` here must therefore never be coerced to 0 on its way to the
  /// database — see `numeric`.
  insertions?: number
  deletions?: number
  commits?: number
  /// The thirteen buckets under the row, base64 of the wire's 66 bytes.
  ///
  /// Stored and forwarded as the string it arrives as. Nothing in this service
  /// decodes it: it is 88 characters of opaque history whose only reader is the
  /// widget, and a relay that parsed it would be a third copy of an encoding
  /// that already has two ends — `farcooler_core::trace::Trace::encode` and
  /// `AgentKit.ActivityTrace`.
  trace?: string
}

interface Device {
  platform: string
  push_token: string
  environment: string | null
  live_activity_start_token: string | null
  /// 0 when this device has turned "When an agent finishes or fails" off. 1 or
  /// NULL both mean notify — NULL is every row that predates migration 0007 and
  /// every build too old to send the field.
  notify_on_done: number | null
}

/// The machine holding this token, or the response to send instead.
///
/// The daemon half of `requireAccount`, and deliberately shaped the same way:
/// the two credentials in this service are not interchangeable — a session says
/// WHO, a daemon token says WHICH MACHINE, and it names an account only because
/// a signed-in person once paired it. Every route that trusts a machine goes
/// through here, so there is one place that decides what a machine is.
///
/// Expiry checked in the WHERE clause, not after: a token that has run out is a
/// token that does not exist. NULL means a pairing issued before expiries
/// existed, which keeps working — logging those machines out to introduce a
/// policy would break a feature people had just set up.
///
/// `label` comes along because it is the machine's name, which is one of the
/// three things a Live Activity's card says.
async function requireDaemon(
  request: Request,
  env: Env,
): Promise<{ id: string; account_id: string; label: string } | Response> {
  const header = request.headers.get('authorization') ?? ''
  const token = header.startsWith('Bearer ') ? header.slice(7) : ''
  if (!token) return json({ error: 'unauthorized' }, 401)

  const daemon = await env.DB.prepare(
    `SELECT id, account_id, label FROM daemons
     WHERE token_hash = ? AND (expires_at IS NULL OR expires_at > ?)`,
  )
    .bind(await sha256(token), Date.now())
    .first<{ id: string; account_id: string; label: string }>()
  if (!daemon) return json({ error: 'unauthorized' }, 401)
  return daemon
}

async function notify(request: Request, env: Env): Promise<Response> {
  const daemon = await requireDaemon(request, env)
  if (daemon instanceof Response) return daemon

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
    `SELECT platform, push_token, environment, live_activity_start_token, notify_on_done
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
  // as a success: the 200 is what it checks, not the count. The same is now true
  // of a `done` notify where every device has opted out, and for the same
  // reason — nothing is wrong, nobody wanted to hear it.
  let delivered = 0
  if (body.status !== 'working') {
    for (const device of devices.results ?? []) {
      // "When an agent finishes or fails", off. Per device inside the loop and
      // not per request outside it, because one account can hold devices that
      // disagree: a phone that should stay quiet and a watch that should not.
      //
      // Only on `done`. A `blocked` agent has stopped and is waiting, which is
      // the other toggle's business and the reason this product exists — reading
      // this column on any other branch would take failures away from someone
      // who only silenced the endings.
      if (body.status === 'done' && device.notify_on_done === 0) continue
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
  //
  // Every device, including one that just had its alert skipped. `done` is the
  // only thing that has ever taken a live card down — see `watch.rs` — so a
  // device that silenced the banner and kept the card would be left with a lock
  // screen reading "Working" over an agent that stopped ten minutes ago. Skip
  // the alert, never the card.
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

/// How many terminals one retirement request may name.
///
/// A runner sweeps its whole fleet the first time it can account for each
/// terminal after starting, so the largest honest request is one id per pane on
/// the machine. A hundred is several times the biggest fleet anyone runs and
/// still small enough that this route cannot become a way to make the worker
/// spend a minute reading one caller's body.
///
/// It is a bound and not a truncation the caller has to notice: the runner sends
/// its sweep in requests of this size — see `push::RETIRE_BATCH` — so a fleet
/// past the bound arrives as two requests rather than as a card nobody mentions
/// again. That agreement matters more than it did: there is one card now, so a
/// sweep whose ids were silently cut past this bound could drop the ONE id that
/// names its leader rather than one of many.
///
/// It no longer has anything to do with D1's hundred-parameter statement limit,
/// which is what bounded it when this route ran a query per named terminal. The
/// query is now one read of one row and the ids are matched in memory.
const RETIRE_LIMIT = 100

/// Take down the card if the run behind it has ended.
///
/// The counterpart to `/v1/notify`, under the same path because it carries the
/// same credential and no other route does: a machine token, which names an
/// account and nothing else. It is a different VERB on the same table — notify
/// says what happened, this says what is no longer happening — and it exists
/// because only one side of this pair knows each half of the answer. The relay
/// knows which card is up, in `install_cards`; only the runner knows whether
/// there is still a run behind one, and a card whose run has gone has nobody
/// left to end it.
///
/// Two ways that used to happen, both of which left a card on the lock screen
/// reading "Waiting for your answer" for an agent that was not waiting:
///
///   - the terminal went away while its card was up. `Watcher::sample` drops
///     the state it holds for a terminal that has left the fleet, and the
///     transition that would have sent `done` can no longer be observed, because
///     there is nothing left to observe it on.
///   - the daemon restarted. Its map of what each terminal was doing is rebuilt
///     empty, so the SAME transition never happens: the agent that was blocked
///     before the restart is simply the agent it first sees, and a first sighting
///     is not a change. Every runner update orphaned every card that was up.
///
/// **The runner still names TERMINALS, and that has not changed with the card.**
/// It names them because it is the side that knows whether a run is still behind
/// one, and it has no idea which of them the card is currently about. What
/// changed is the answer: with one card per install there is exactly one
/// terminal in this list that can take a card down — the one the card is
/// LEADING with. Naming any of the others is naming a terminal this card was
/// never about, and ending it would clear the lock screen of an agent that is
/// still running because a different agent stopped.
///
/// That is also what preserves the two behaviors built on this route:
///
///   - the four orphan paths from the sweep above all name the terminal whose
///     run has gone. If the card was leading with it, it comes down; if it was
///     not, the card was already about somebody else and is still true.
///   - `Done` for an agent the person is demonstrably watching retires rather
///     than notifies, so that suppressing its alert does not leave "Working"
///     sitting over a stopped agent. Same rule, same outcome: if that agent led
///     the card the card goes, and if it did not, the card belongs to another
///     agent and stays. The next `working` push from any surviving agent starts
///     a fresh card within a tick either way.
///
/// Silent, and that is the point of not folding it into `/v1/notify`. Nothing
/// here alerts: no device push goes out, and the activity push carries no alert
/// dictionary. A card coming down is not news — the person either closed the
/// pane themselves or updated their runner — and a buzz per orphaned card on a
/// restart would be this feature interrupting somebody to announce its own
/// housekeeping.
///
/// A card it cannot ADDRESS is deleted and left to the `stale-date` its start
/// carried, exactly as `done` does, and for the same reason: an update token
/// exists only once the app has run and reported it, and no amount of asking
/// makes one out of a card that has never been addressable. The row goes either
/// way, because the row's whole meaning is "a card the relay believes is up" —
/// and keeping one for a card nothing can reach would refuse this install a card
/// for every run that followed.
async function retireActivities(request: Request, env: Env): Promise<Response> {
  const daemon = await requireDaemon(request, env)
  if (daemon instanceof Response) return daemon

  const body = await request.json<{ terminals?: unknown }>()
  // A 400 rather than a shrug, unlike the unknown `status` in `pushActivity`:
  // there is no forward-compatibility story to protect here, because a request
  // that names no terminals is asking for nothing at all.
  if (!Array.isArray(body.terminals)) return json({ error: 'terminals' }, 400)
  const terminals = body.terminals
    .filter((terminal): terminal is string => typeof terminal === 'string' && terminal !== '')
    .slice(0, RETIRE_LIMIT)
  if (terminals.length === 0) return json({ retired: 0 })

  // The runs behind these terminals are over, so their rows leave the roster.
  //
  // This has to happen whether or not a card comes down. A row nothing retires
  // would go on being counted in the header — "3 in flight" over a runner that
  // restarted an hour ago — until `ROW_RETENTION_MS` forgot it, and the whole
  // reason this route exists is that only the runner knows a run has ended.
  //
  // Scoped to the account the token names, the same as every read a machine can
  // reach: a runner says which terminals, never whose. Two runners cannot mint
  // the same UUID, but the account clause is what makes that a fact about this
  // query rather than a fact about UUIDs.
  // In chunks, because D1 refuses a statement with more than a hundred bound
  // parameters and `RETIRE_LIMIT` is a hundred on its own. That limit is the one
  // this route used to iterate around and stopped needing when it became a
  // single-row read; it is back for this DELETE, so it is honored explicitly
  // rather than being a thing the largest honest sweep discovers in production.
  const PARAMETERS = 90
  for (let at = 0; at < terminals.length; at += PARAMETERS) {
    const chunk = terminals.slice(at, at + PARAMETERS)
    await env.DB.prepare(
      `DELETE FROM live_activities
       WHERE account_id = ? AND terminal IN (${chunk.map(() => '?').join(',')})`,
    )
      .bind(daemon.account_id, ...chunk)
      .run()
  }

  const running = await env.DB.prepare(
    `SELECT update_token, environment, leader_terminal FROM install_cards
     WHERE account_id = ?`,
  )
    .bind(daemon.account_id)
    .first<{ update_token: string; environment: string | null; leader_terminal: string | null }>()
  if (!running) return json({ retired: 0 })

  // **What ends a card is an empty fleet, not a named leader.**
  //
  // It used to be the leader's name in this list, because a card could only be
  // about one agent and losing that agent lost the card. With a row each, a
  // retired terminal is a row leaving the roster — already done above — and the
  // card is still true about everybody else. So the card comes down when there
  // is nobody left for it to be about, and the sweep of a runner that restarted
  // no longer clears the lock screen of three agents on another runner that did
  // not.
  //
  // A card whose HEADLINE was retired while others are still running is left
  // where it is rather than re-composed here. It is naming an agent that has
  // stopped, for as long as it takes any surviving agent to push — ten seconds
  // at most, since a working agent refreshes its card on that clock. Silently
  // re-pushing here would mean this route composing a card state, which is the
  // one thing it has never done: it says what is no longer happening, and
  // `/v1/notify` says what is.
  const left = await readFleet(env, daemon.account_id, Date.now())
  if (left.some(row => row.status === 'blocked' || row.status === 'working')) {
    return json({ retired: 0 })
  }

  if (running.update_token !== TOKEN_UNKNOWN) {
    await deliverActivity(env, daemon.account_id, running.update_token, running.environment, {
      event: 'end',
      // Nothing to say, because nothing is being reported: the card comes down
      // at once — see `Dismissal` — so this state exists only because the app's
      // `ContentState` has to decode, and a detail line would be a sentence
      // about a run the runner has just said it cannot account for. The leader's
      // own fields go out empty for the same reason: there is no leader left to
      // name, and the card is gone before anything could be read off it.
      state: { terminal: '', label: '', machine: '', status: 'done', detail: '' },
      dismissal: 'immediate',
    })
  }
  await env.DB.prepare(`DELETE FROM install_cards WHERE account_id = ?`)
    .bind(daemon.account_id)
    .run()

  // What was actually taken down, not what was asked about. A runner sweeps
  // every terminal it cannot account for and at most one of them is the card's
  // leader, so this is 0 or 1 where it used to be a tally — which is the number
  // worth logging either way.
  return json({ retired: 1 })
}

/// The `update_token` of a card the relay started while the app was not running.
///
/// A row HAS to exist the moment a card is push-started, or the next push finds
/// none and starts another card. The daemon sends `working` about every ten
/// seconds for the length of a run, so a half-hour run would leave on the order
/// of a hundred and eighty cards on the lock screen — none of which the relay
/// holds an update token for, and none of which it can therefore ever end. This
/// row is what `UNIQUE (account_id)` refuses the second start against, which is
/// the invariant the comment on that constraint already claims.
///
/// The empty string rather than NULL because `install_cards.update_token` is
/// declared NOT NULL, the migrations in this service are additive only — see the
/// header of 0006 for why — and SQLite cannot loosen a column in place.
///
/// It is NOT a token and must never reach APNs: an activity push addressed to
/// the empty string addresses no activity. Every read of `update_token` has to
/// ask whether it is this first, and the one place that writes a real one,
/// `/v1/devices/activity`, already 400s an empty `updateToken` — so the app
/// cannot report this value by accident.
const TOKEN_UNKNOWN = ''

/// How long an unaddressable row may hold this install's one card slot.
///
/// **One bound where there were two, because they had become the same bound.**
/// `DISMISSAL_MEMORY_MS` stood beside this and held a swipe in memory for an
/// hour so a dismissed card could not come back within ten seconds. That job is
/// done differently now: a swipe is answered by a `blocked` push and by nothing
/// else, because `working` no longer starts cards at all, and the escalation
/// below clears the dismissal as it raises the replacement. What was left of the
/// old constant was "forget a refusal that has aged out and free the slot" —
/// which is this, read off the same `updated_at` that a dismissal stamps. Two
/// constants of the same value, doing overlapping jobs, is exactly the drift a
/// number is supposed to avoid.
///
/// What this covers, then, is every way a row can outlive the card it stands
/// for:
///
///   - **a start APNs accepted that the phone never rendered.** `startCard`
///     writes its row only after an accepted push now, which closes the case of
///     a REFUSED start holding the slot; acceptance is not rendering, and an app
///     that is never opened never files the update token that would make the
///     card addressable. A row stuck that way used to refuse the install a card
///     for the rest of time, silently — and `updated_at`, written in four places
///     and read in none, was sitting right there answering nobody. This is the
///     read it was missing.
///   - **a swipe on a fleet that never blocks again.** The person refused the
///     card, nothing has asked them a question since, and the row is holding a
///     slot for a card that is gone.
///
/// An hour, which is `STALE_AFTER_S` in `services/relay/src/push.ts` and the
/// same number for the same reason: after that long the relay does not know
/// whether the card it believes in is on any lock screen, and a card marked
/// stale that nothing can move is worth less than the chance to start a fresh
/// one.
const CLAIM_MEMORY_MS = 60 * 60 * 1000

/// How long a row stays in an account's roster before it is forgotten.
///
/// Twenty-four hours, and the number is the design's own rather than a round
/// one. A row's trace snaps to the shortest window that contains its activity —
/// 1h, 6h or 24h — so past a day it cannot contribute to any window the card can
/// draw, and it has nothing left to say. Purging at the design's own maximum is
/// the smallest number that loses nothing visible.
///
/// Applied LAZILY, on write. There are no cron triggers in this relay, so there
/// is nowhere else to put it; and doing it per account on the account's own
/// notice means the work is proportional to what is actually running.
const ROW_RETENTION_MS = 24 * 60 * 60 * 1000

/// How long a row keeps a LINE on the card before it collapses into `+N more`.
///
/// Deliberately `STALE_AFTER_S` again: a row goes quiet exactly when the card as
/// a whole would be marked out of date, so there is one number to reason about
/// rather than two that drift apart. A quiet row is still in the fleet — still
/// counted in the header and in the totals — it just stops spending one of the
/// few lines the card has on an agent that has said nothing for an hour.
const ROW_QUIET_AFTER_MS = 60 * 60 * 1000

/// How many agents the card draws a line for.
///
/// **Four, and the ceiling it is measured against is not four.** The binding
/// limit here is the card's height, not the payload: a Live Activity's lock
/// screen presentation is a few lines tall, and the design answers that with
/// `+N more` rather than by growing.
///
/// The byte arithmetic, because "measured" has to mean measured. A row encodes
/// to 341 bytes of JSON typically and 399 at its worst — a 36-character UUID, a
/// 24-character label and runner name, a 40-character detail line, three counts
/// and 88 characters of base64 trace. The card's fixed part — headline, header
/// counts, totals, the `aps` envelope and an alert — is 689 bytes at its worst.
/// So `(4096 - 689) / 399` is **8 rows** against the APNs payload cap, and
/// ActivityKit's separate 4KB cap on the content state alone allows 9. Eight is
/// the real maximum and four is a design choice inside it, which is the opposite
/// of the manifest ceiling that was written as fifteen and measured at four.
///
/// `STATE_BUDGET` is what enforces the measurement rather than trusting it.
const ROWS_SHOWN = 4

/// The most an encoded content state may reach, in bytes.
///
/// ActivityKit caps a content state at 4KB and APNs caps the whole payload the
/// same way, and neither truncates: a card over the line is REFUSED, which looks
/// from every side like a relay that sent nothing. The arithmetic above says
/// four rows cannot reach this — but the arithmetic assumes bounded fields, and
/// every bound in it belongs to a runner that ships separately from this worker.
/// So rows are added until one would cross this line and then no more, which
/// costs a fleet card its last row in the worst case and never costs it the card.
///
/// Three kilobytes rather than four: the state is the largest part of the
/// payload and not all of it, and the headroom is the envelope, the alert and the
/// attributes a start also carries.
///
/// Exported because that last sentence is arithmetic and nothing was checking
/// it. `STATE_BUDGET` bounds the state; the cap APNs applies is on the whole
/// payload, and a budget raised to fill the cap on its own would put every
/// alerting push over it — silently, since a refused push is indistinguishable
/// from a relay that sent nothing. See `leaves room in the payload for the alert
/// and the envelope` in `test/relay.test.ts`, which measures the envelope off a
/// real start and adds it up.
export const STATE_BUDGET = 3 * 1024

/// The most an activity push's alert may spend of that headroom.
///
/// **The alert was unbounded and it is on the same 4KB payload.** Nothing here
/// ever measured it: `title` and `subtitle` are composed on the runner and cut
/// there — `feed::WIDTH` is forty characters and `SAID_WIDTH` a hundred and
/// twenty — so the arithmetic worked out for every real notice and the cap was
/// never the relay's problem. But every one of those bounds belongs to a
/// program that ships separately from this worker, and the failure if one moves
/// is not a long banner: APNs refuses the whole push, and a refused activity
/// push looks from every side like a relay that sent nothing.
///
/// The numbers are generous against what a lock screen can draw and mean against
/// what would break the cap. A banner shows a title on one line and a body on
/// about two, so 128 and 512 bytes are past the point where iOS is already
/// eliding — a cut here can only remove text the person was never shown.
///
/// Exported for the same reason `STATE_BUDGET` is: these are the other half of
/// the payload arithmetic, and the sum of the two halves is what has to fit.
export const ALERT_TITLE_BUDGET = 128
export const ALERT_BODY_BUDGET = 512

/// `text`, cut to at most `bytes` of UTF-8, never mid-character.
///
/// Bytes and not characters, because the cap is bytes: a card carrying an
/// agent's own words can be three bytes a character, and cutting at a hundred
/// and twenty of those is nearly four hundred. `Intl.Segmenter` would be more
/// correct about grapheme clusters and is not worth it here — the worst a code
/// point boundary can do is separate an emoji from its modifier at the very end
/// of a line that was already too long to read.
function cut(text: string, bytes: number): string {
  const encoder = new TextEncoder()
  if (encoder.encode(text).length <= bytes) return text
  let out = ''
  let size = 0
  // `for...of` iterates code points rather than UTF-16 units, so a surrogate
  // pair is never split in half — which would produce a lone surrogate, and
  // `JSON.stringify` writes that as an escape the app decodes to a replacement
  // character.
  for (const character of text) {
    const width = encoder.encode(character).length
    if (size + width > bytes) break
    out += character
    size += width
  }
  return out
}

/// The shortest interval between two pushes that are only about volume.
///
/// A fleet card changes whenever ANY agent changes, which is strictly more
/// updates than a card about one agent was. Four busy agents pushing every ten
/// seconds is the exact case the old `leads` was protecting against — it did so
/// by refusing three of them the card entirely, which is also why one wedged
/// agent could silence a whole fleet. That protection does not disappear now
/// that rows exist; it moves here, where being wrong costs a number that is ten
/// seconds out of date rather than an agent nobody hears about.
///
/// A status change is news and goes at once: `blocked` and `done` are never
/// held. `+142 −37` becoming `+147 −37` waits.
const COALESCE_MS = 10 * 1000

/// One agent's row, as the relay stores it. See migration 0008.
interface AgentRow {
  terminal: string
  label: string | null
  machine: string | null
  status: string | null
  detail: string | null
  insertions: number | null
  deletions: number | null
  commits: number | null
  trace: string | null
  started_at: number | null
  status_since: number | null
  updated_at: number
}

/// Which tier a row sorts into. Lower is more urgent.
///
/// The same precedence the rest of the product uses: an agent waiting on a
/// person outranks one waiting to be read, which outranks one that needs
/// nobody. A status this relay does not recognize sorts last rather than being
/// refused — the daemon ships separately and will eventually send one invented
/// after this code was written, and a card that dropped that agent would be
/// worse than one that draws it at the bottom.
function tier(status: string | null): number {
  if (status === 'blocked') return 0
  if (status === 'done') return 1
  if (status === 'working') return 2
  return 3
}

/// Whether a row still has anything to say about now.
///
/// Only rows in a tier the card draws, and only ones that have spoken inside
/// `ROW_QUIET_AFTER_MS`. Both halves matter: a quiet row is dropped from the
/// LINES but kept in the counts, and this is the test the lines use.
function speaks(row: AgentRow, now: number): boolean {
  return tier(row.status) < 3 && now - row.updated_at < ROW_QUIET_AFTER_MS
}

/// What the card says about an account, derived on every push and stored
/// nowhere.
///
/// **Derived and not stored**, which is the same argument that deleted the
/// `line` field from `ContentState`: a stored header is a second copy of a
/// number the rows already answer, and two writers that can disagree about one
/// fact will. It is a sum over rows the relay just read, every time.
interface Fleet {
  /// Every row the relay holds for the account, ordered as the card draws them.
  all: AgentRow[]
  /// The ones that get a line. See `ROWS_SHOWN` and `STATE_BUDGET`.
  shown: AgentRow[]
  blocked: number
  review: number
  working: number
  insertions: number | null
  deletions: number | null
  commits: number | null
}

/// Forget this account's rows that have nothing left to say, then read the rest.
///
/// Purging and reading in one place because they are one thought: a row past
/// `ROW_RETENTION_MS` must not reach a card, and the cheapest way to guarantee
/// that is for the only reader to have deleted it first. Per ACCOUNT, on that
/// account's own notice, which is what makes a lazy purge proportional — a
/// person with no runners running costs nothing to keep.
async function readFleet(env: Env, account: string, now: number): Promise<AgentRow[]> {
  await env.DB.prepare(`DELETE FROM live_activities WHERE account_id = ? AND updated_at < ?`)
    .bind(account, now - ROW_RETENTION_MS)
    .run()

  const rows = await env.DB.prepare(
    `SELECT terminal, label, machine, status, detail, insertions, deletions, commits,
            trace, started_at, status_since, updated_at
     FROM live_activities WHERE account_id = ?`,
  )
    .bind(account)
    .all<AgentRow>()
  return rows.results ?? []
}

/// Write down what one notice said about one agent.
///
/// **The relay keeps this and it did not used to keep anything**, which is a
/// real change to a rule stated at length on `/v1/notify` and is why it is
/// spelled out here as well as in the migration. What is kept is exactly what a
/// row draws — a name, a runner, a tier, one composed line, three counts and a
/// trace — for at most a day, per account, purged by the next notice that
/// arrives. Nothing is logged, no body is written anywhere else, and the totals
/// and the header are derived from these rather than stored beside them.
///
/// `status_since` moves only when the tier actually moves, because it is the
/// ordering key: rows sort longest-waiting first within a tier, and stamping it
/// on every notice would sort by who spoke last instead — which is the opposite
/// rule, and it would let a busy agent take the headline from a blocked one by
/// being chatty.
///
/// Counts COALESCE rather than overwrite. A notice that measured nothing this
/// tick has not un-measured what the last one found; absent means "no new
/// answer", and the row keeps the last real one until the runner has another.
async function rememberAgent(
  env: Env,
  account: string,
  machine: string,
  terminal: string,
  status: string,
  state: ActivityState,
  body: Notification,
  prior: AgentRow | undefined,
  now: number,
): Promise<AgentRow> {
  // Built ONCE and used twice: bound into the statement below, and returned to
  // the caller as the row the card composes from.
  //
  // It was written out twice — here and again at the call site — and the two
  // copies could disagree without anything failing, because the card is composed
  // from the caller's copy and only the NEXT request ever reads the column. A
  // rule like `status_since` moving only on a real tier change would then hold
  // for the card in front of the person and not for the row underneath it, and
  // the drift would show up as a card that reorders itself when nothing changed.
  const mine: AgentRow = {
    terminal,
    label: state.label,
    machine,
    status,
    detail: state.detail,
    insertions: numeric(body.insertions) ?? prior?.insertions ?? null,
    deletions: numeric(body.deletions) ?? prior?.deletions ?? null,
    commits: numeric(body.commits) ?? prior?.commits ?? null,
    trace: trace(body.trace) ?? prior?.trace ?? null,
    started_at: state.startedAt ?? null,
    // Only when the tier actually moves. See above.
    status_since: prior && prior.status === status ? (prior.status_since ?? now) : now,
    updated_at: now,
  }

  await env.DB.prepare(
    `INSERT INTO live_activities
       (id, account_id, terminal, update_token, environment, updated_at,
        label, machine, status, detail, insertions, deletions, commits, trace,
        started_at, status_since)
     VALUES (?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (account_id, terminal)
     DO UPDATE SET updated_at = excluded.updated_at,
                   label = excluded.label,
                   machine = excluded.machine,
                   status = excluded.status,
                   detail = excluded.detail,
                   insertions = COALESCE(excluded.insertions, live_activities.insertions),
                   deletions = COALESCE(excluded.deletions, live_activities.deletions),
                   commits = COALESCE(excluded.commits, live_activities.commits),
                   trace = COALESCE(excluded.trace, live_activities.trace),
                   started_at = excluded.started_at,
                   status_since = excluded.status_since`,
  )
    .bind(
      crypto.randomUUID(),
      account,
      terminal,
      // Not an address and never one. A roster row is about an agent; the one
      // card's address lives on `install_cards`. The column is NOT NULL from
      // 0003 and SQLite cannot loosen one in place, so the sentinel that already
      // means "not an address" is what an additive migration has. See
      // `TOKEN_UNKNOWN` and the header of 0008.
      TOKEN_UNKNOWN,
      mine.updated_at,
      mine.label,
      mine.machine,
      mine.status,
      mine.detail,
      // The count this notice measured, not the one carried forward: `COALESCE`
      // above is what carries it, and binding the carried value would make the
      // statement's own rule unobservable.
      numeric(body.insertions),
      numeric(body.deletions),
      numeric(body.commits),
      trace(body.trace),
      mine.started_at,
      mine.status_since,
    )
    .run()
  return mine
}

/// A trace the daemon actually sent, or NULL.
///
/// Opaque and bounded, which is the whole of what this service knows about it: a
/// trace is 88 characters of base64 by construction — thirteen buckets of two
/// channels plus the axis — and the bound is here so a caller cannot make this
/// column, and through it the APNs payload, any size it likes.
function trace(value: unknown): string | null {
  return typeof value === 'string' && value ? value.slice(0, 128) : null
}

/// A count the daemon actually sent, or NULL.
///
/// The one place absent-is-not-zero is enforced, and it is a function rather
/// than an inline check because getting it wrong is invisible: `Number(undefined)`
/// is NaN, `undefined || 0` is 0, and either would write a confident zero into a
/// column whose whole purpose is to tell "nobody measured this" from "nothing
/// changed". A card drawing `+0 −0` over a worktree nobody probed is stating a
/// measurement that was never made.
///
/// Type-checked rather than trusted for the same reason `version` and
/// `startedAt` are: the body is whatever a machine posted.
function numeric(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
    ? Math.min(Math.floor(value), 0xffffffff)
    : null
}

/// Order the rows, count the tiers, and total what they changed.
///
/// Ordering is `tier` then longest-waiting, so the agent that needs a person
/// never falls off the bottom and a chatty one cannot climb past a stuck one.
/// Ties break on terminal, because `all()` does not promise an order and a card
/// whose rows shuffled between two identical pushes would be a card that
/// flickers for no reason.
///
/// The counts are over EVERY row and the lines are over the first few, which is
/// the whole point of `+N more`: a header that counted only what fits would say
/// "2 need you" while three agents were waiting.
function composeFleet(rows: AgentRow[], now: number): Fleet {
  const all = [...rows].sort((a, b) => {
    const byTier = tier(a.status) - tier(b.status)
    if (byTier !== 0) return byTier
    const bySince = (a.status_since ?? a.updated_at) - (b.status_since ?? b.updated_at)
    if (bySince !== 0) return bySince
    return a.terminal < b.terminal ? -1 : a.terminal > b.terminal ? 1 : 0
  })

  let insertions: number | null = null
  let deletions: number | null = null
  let commits: number | null = null
  // Absent stays absent all the way up. A fleet where nobody has measured
  // anything reports no totals rather than `+0 −0`, and one where a single row
  // has numbers reports that row's — which is the honest sum of what is known.
  for (const row of all) {
    if (row.insertions !== null) insertions = (insertions ?? 0) + row.insertions
    if (row.deletions !== null) deletions = (deletions ?? 0) + row.deletions
    if (row.commits !== null) commits = (commits ?? 0) + row.commits
  }

  return {
    all,
    shown: all.filter(row => speaks(row, now)).slice(0, ROWS_SHOWN),
    blocked: all.filter(row => row.status === 'blocked').length,
    review: all.filter(row => row.status === 'done').length,
    working: all.filter(row => row.status === 'working').length,
    insertions,
    deletions,
    commits,
  }
}

/// The header, in the words the lock screen shows: `2 need you · 3 in flight`.
///
/// Composed here and not on the phone ONLY because it is also the alert on a
/// start, and an alert is a string APNs carries rather than a state the card
/// renders. The card gets the three numbers and writes its own sentence — see
/// `ActivityState.blocked` — so this is not a second copy of the card's wording,
/// it is the one place a sentence is genuinely required.
///
/// Empty clauses are dropped rather than written as zero. "0 need you" is worse
/// than silence on a lock screen, and a fleet with nothing in any tier gets a
/// fallback rather than an empty string, because an alert with no title is an
/// alert iOS may draw as a blank banner.
function fleetHeader(fleet: Fleet): string {
  const parts: string[] = []
  if (fleet.blocked > 0) parts.push(`${fleet.blocked} need${fleet.blocked === 1 ? 's' : ''} you`)
  if (fleet.review > 0) parts.push(`${fleet.review} to review`)
  if (fleet.working > 0) parts.push(`${fleet.working} in flight`)
  return parts.length > 0 ? parts.join(' · ') : 'Your agents'
}

/// Fill in the fleet half of a card's state, up to the byte budget.
///
/// The headline is already on `state` and is left alone: an app too old to know
/// about rows reads exactly what it always read, and one that knows about them
/// draws the first row and the headline as the same thing.
///
/// Rows are added one at a time and measured as they go. `STATE_BUDGET` is not
/// belt and braces — every length bound in the arithmetic behind `ROWS_SHOWN`
/// belongs to a runner that ships separately from this worker, so a build that
/// widened one of them could otherwise put this payload over the cap, where APNs
/// does not truncate it but refuses it outright.
function withFleet(state: ActivityState, fleet: Fleet): ActivityState {
  state.blocked = fleet.blocked
  state.review = fleet.review
  state.working = fleet.working
  if (fleet.insertions !== null) state.insertions = fleet.insertions
  if (fleet.deletions !== null) state.deletions = fleet.deletions
  if (fleet.commits !== null) state.commits = fleet.commits

  const rows: ActivityRow[] = []
  for (const row of fleet.shown) {
    rows.push({
      terminal: row.terminal,
      label: row.label ?? '',
      machine: row.machine ?? '',
      status: row.status ?? '',
      detail: row.detail ?? '',
      ...(row.insertions !== null ? { insertions: row.insertions } : {}),
      ...(row.deletions !== null ? { deletions: row.deletions } : {}),
      ...(row.commits !== null ? { commits: row.commits } : {}),
      ...(row.started_at !== null ? { startedAt: row.started_at } : {}),
      updatedAt: row.updated_at,
      ...(row.trace ? { trace: row.trace } : {}),
    })
    if (new TextEncoder().encode(JSON.stringify({ ...state, rows })).length > STATE_BUDGET) {
      rows.pop()
      break
    }
  }
  state.rows = rows
  // Everybody the card has no line for, which is the fleet minus the lines and
  // not the fleet minus `ROWS_SHOWN`: a row dropped for the byte budget, and a
  // row that has gone quiet, are both agents this card is not naming.
  state.more = Math.max(0, fleet.all.length - rows.length)
  return state
}

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

  // No terminal, no activity. The card leads with one and puts it in the URL a
  // tap opens, and a leader under the empty string is a card that cannot say
  // which agent it is about and cannot be retired by the runner that knows.
  const terminal = body.terminal ?? ''
  if (!terminal) return

  const state: ActivityState = {
    // The leader, which is the card's whole content now that the card is per
    // install. See `ActivityState` in `push.ts` for why these moved off the
    // attributes.
    terminal,
    // The agent's name if the daemon sent one, and the notification's title if
    // it did not. A blank line on the card would be worse than repeating a line
    // the person has already read.
    label: body.label || body.title,
    machine: daemon.label,
    status,
    // The daemon's own words when it has any. When it has none, a blocked card
    // still needs to say what it is waiting for — "Needs You" alone does not —
    // but a finished one does not, because the card already reads "Finished"
    // above this line and repeating it is the sort of thing you cannot unsee.
    detail: body.subtitle || (status === 'blocked' ? 'Waiting for your answer' : ''),
    // The leader's turn clock, on EVERY push rather than only the start.
    //
    // It rode the attributes when the card was about one terminal, so it could
    // only ever be sent once; a card whose leader changes has to be able to
    // change the clock with it, or the second agent's work counts from the
    // first agent's start.
    //
    // Type-checked rather than trusted, the same way `version` is elsewhere. The
    // body is whatever a machine posted, and the app decodes this field as a
    // number — a string reads back as nil there and costs the timer silently, so
    // a `null` or a `"1755..."` is dropped here where it is still visible rather
    // than on a lock screen where it is not.
    startedAt: typeof body.startedAt === 'number' ? body.startedAt : undefined,
  }
  // Everything this account has running, with what just happened folded in.
  //
  // Read BEFORE the write so the previous status is still here to compare
  // against: `status_since` must move only when the tier actually moves, and the
  // coalescing below has to know whether this notice is news or arithmetic. One
  // read either way — the card needs every row to compose a header, and the row
  // this notice is about is one of them.
  const now = Date.now()
  const before = await readFleet(env, daemon.account_id, now)
  const prior = before.find(row => row.terminal === terminal)
  // The row the write just produced, handed back rather than read again: it is
  // the same object the statement was bound from, so the card cannot disagree
  // with the column underneath it.
  const mine = await rememberAgent(
    env, daemon.account_id, daemon.label, terminal, status, state, body, prior, now,
  )
  const fleet = composeFleet([...before.filter(row => row.terminal !== terminal), mine], now)

  // The headline, which is what `leads` used to decide and no longer does.
  //
  // **That function was a GATE and this is a sort.** It answered "may this agent
  // appear on the card at all", and answering no is why one wedged leader could
  // silence a whole fleet: a working agent that was not the leader pushed
  // nothing, so four busy agents behind one stuck one were invisible. With a row
  // each, the only question left is which row goes on top and where a tap lands
  // — the same precedence, blocked over to-review over working and
  // longest-waiting first within a tier, but being wrong now costs a misplaced
  // tap instead of a silent product.
  //
  // The headline is the first row rather than a remembered leader, so it needs
  // nothing carried between requests to be right. `install_cards.leader_*` is
  // still written, because the dismissal escalation below asks what tier the
  // card is currently showing — a different question, about the card rather than
  // about the fleet.
  const headline = fleet.shown[0] ?? mine
  if (headline.terminal !== terminal) {
    state.terminal = headline.terminal
    state.label = headline.label ?? ''
    state.machine = headline.machine ?? ''
    state.status = (headline.status ?? status) as ActivityState['status']
    state.detail = headline.detail ?? ''
    state.startedAt = headline.started_at ?? undefined
  }
  withFleet(state, fleet)

  // What separates the tiers is the alert, not whether a push goes out at all.
  // An activity push carrying an alert dictionary is PRESENTED — lock screen
  // banner, Apple Watch haptic — and one without it changes the card in place
  // and says nothing. `blocked` has earned that interruption and `done` closes
  // it out; `working` never earns it, at any point in the card's life. A banner
  // every ten seconds for an agent that is merely busy is the notification
  // people switch the app off over, which then costs them the one push this
  // whole product exists to deliver.
  // Cut to the payload's budget, never to a reader's taste. See `ALERT_TITLE_BUDGET`.
  const alert =
    status === 'working'
      ? undefined
      : {
          title: cut(body.title, ALERT_TITLE_BUDGET),
          body: cut(body.subtitle ?? '', ALERT_BODY_BUDGET),
        }

  const running = await env.DB.prepare(
    `SELECT update_token, environment, leader_terminal, leader_status, dismissed_at,
            updated_at, pushed_at
     FROM install_cards WHERE account_id = ?`,
  )
    .bind(daemon.account_id)
    .first<{
      update_token: string
      environment: string | null
      leader_terminal: string | null
      leader_status: string | null
      dismissed_at: number | null
      updated_at: number
      pushed_at: number | null
    }>()

  // Whether the fleet still has anything the card is FOR.
  //
  // This is what ends a card now, and it is a different question from the one
  // `done` used to answer. A `done` from the agent the card was leading with
  // took the whole card down while three other agents were still running,
  // because the card could only ever be about one of them; with a row each, an
  // agent finishing is a row changing tier and the card is still true. So the
  // card lives while anything is blocked or working, and ends when the last of
  // them stops.
  //
  // A `done` row stays in the roster and keeps being counted as "to review" —
  // that is the tier it is in, and it is what the header's middle number means.
  // It just cannot, on its own, keep a card on the lock screen.
  const alive = fleet.all.some(row => row.status === 'blocked' || row.status === 'working')

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

    // Every agent has stopped, so the card has nothing left to be about, and
    // the row goes with it either way: the update token dies with the activity
    // it was issued for, and a row left behind would refuse this install a card
    // for every run that follows. An unaddressed card is left to the
    // `stale-date` its start carried, which is the bounded hole push-to-start
    // has always had — better than a permanent one.
    //
    // The last state stays up for `DISMISSAL_DELAY_S`, so that somebody who
    // picks the phone up because of the alert has the last word to read when
    // they get there.
    if (!alive) {
      if (address) {
        await deliverActivity(env, daemon.account_id, address, running.environment, {
          event: 'end',
          state,
          alert,
        })
      }
      await env.DB.prepare(`DELETE FROM install_cards WHERE account_id = ?`)
        .bind(daemon.account_id)
        .run()
      return
    }

    if (!address) {
      // The card cannot be moved, and there is no longer any such thing as a
      // blind card showing the WRONG tier.
      //
      // There used to be, and correcting it was the whole of this branch: a
      // silent `working` start claimed the row, the `blocked` push that followed
      // found a card it could not address, and the lock screen read "Working"
      // beside a banner saying the agent needed an answer. A second card was
      // started to say the true thing and the first was left to expire.
      //
      // A card only starts on `blocked` now, and `startCard` records the
      // headline it started with — which is always the blocked agent, because
      // blocked sorts first. So a blind card is already showing the only tier
      // that can raise one, and the correction it needed has nothing left to
      // correct. What remains here are the three cases where the relay may raise
      // a card it has none for: a new question after a dismissal, a dismissal
      // that has outlived its card, and a claim that has outlived its own
      // credibility.

      // The person swiped the card away and an agent has since blocked.
      //
      // A dismissal is a refusal of what the card was SAYING, not of everything
      // this fleet will ever say, and a question nobody has answered is news
      // they have not seen. So a fresh card goes up — and `startCard`'s conflict
      // arm clears `dismissed_at` on the way, which is what keeps this to
      // exactly one card per dismissal: the next blocked push finds nothing
      // dismissed here and falls through.
      //
      // `working` never takes this path. It is the silent tier, it has no news
      // to correct, and re-raising a card the person swiped away is the thing
      // that made a dismissed card come back within ten seconds.
      if (status === 'blocked' && running.dismissed_at !== null) {
        await startCard(env, daemon, devices, headline.terminal, state, startAlert(fleet, body))
        return
      }

      // A claim that has aged out. See `CLAIM_MEMORY_MS`.
      //
      // `install_cards.updated_at` was written in four places and read in none,
      // so a row whose card never appeared held this install's one slot for
      // good: the relay believed in a card nothing could see and refused to
      // start another. This is the read that column was missing. The row is
      // deleted rather than ignored, because ignoring it would leave the same
      // dead claim there for the next push to trip over.
      if (now - running.updated_at >= CLAIM_MEMORY_MS) {
        await env.DB.prepare(`DELETE FROM install_cards WHERE account_id = ?`)
          .bind(daemon.account_id)
          .run()
        if (status === 'blocked') {
          await startCard(env, daemon, devices, headline.terminal, state, startAlert(fleet, body))
        }
      }
      return
    }

    // Volume moved and nothing else did, and something moved recently enough.
    //
    // The card is already stored — `rememberAgent` ran above — so what is held
    // here is the PUSH and never the state: the next notice inside ten seconds
    // carries these numbers along with whatever it is about, and a card ten
    // seconds behind on `+142 −37` is the trade `COALESCE_MS` names. A tier
    // change is never held, which is why `blocked` and `done` are news by
    // definition and a first sighting of an agent is too.
    const news = !prior || prior.status !== status || status !== 'working'
    if (!news && running.pushed_at !== null && now - running.pushed_at < COALESCE_MS) return

    await deliverActivity(env, daemon.account_id, address, running.environment, {
      event: 'update',
      state,
      alert,
    })

    // Who the card is headlining, and when it was last pushed.
    //
    // `pushed_at` moves on every push because that is what the coalescing clock
    // means; `updated_at` moves only when the headline actually changes, because
    // that one is the age of the row's claim and a card being refreshed is not a
    // claim being re-made. Two clocks, two readers, and neither can be derived
    // from the other — see migration 0008.
    const moved =
      running.leader_terminal !== headline.terminal || running.leader_status !== state.status
    await env.DB.prepare(
      `UPDATE install_cards
       SET leader_terminal = ?, leader_status = ?, pushed_at = ?, updated_at = ?
       WHERE account_id = ?`,
    )
      .bind(
        headline.terminal,
        state.status,
        now,
        moved ? now : running.updated_at,
        daemon.account_id,
      )
      .run()
    return
  }

  // **A card starts on `blocked` and on nothing else.**
  //
  // It used to start on `working` too, silently, so that the card followed a
  // whole run rather than appearing once something had gone wrong. That start
  // could not carry an alert — a banner every time any agent picked up work is
  // the notification people switch the app off over — and **iOS discards a
  // push-to-start activity with no alert dictionary.** Silently, at HTTP 200.
  // So the silent start was not a quieter card, it was no card: the feature
  // people were meant to see on every run turned up two or three times in total.
  //
  // Starting on `blocked` used to look expensive, because it meant giving up the
  // busy-agent card. With a row per agent it costs nothing of the sort: once the
  // card exists it shows every agent, working ones included, so the only case
  // given up is work happening with nobody needed — which is exactly the case
  // that could never have started silently anyway.
  //
  // And at `blocked` there is a real alert to carry. It is the fleet's own
  // header — "2 need you · 3 in flight" — which is news about the fleet rather
  // than a buzz about one agent beginning work, and news is what Apple is asking
  // for when it requires a start to alert.
  if (status !== 'blocked') return

  await startCard(env, daemon, devices, headline.terminal, state, startAlert(fleet, body))
}

/// What a start puts on the lock screen beside the card it raises.
///
/// The header, and under it whatever the blocked agent is asking. iOS requires a
/// start to alert — see `ActivityBase.alert` — so this is not decoration, it is
/// the difference between a card and nothing; and what it says is what makes the
/// requirement legitimate rather than something to work around.
///
/// The header rather than the notice's own title, which is the whole ruling. "2
/// need you · 3 in flight" is a statement about the fleet that a person is
/// entitled to be interrupted by. "claude started working" is not, and that is
/// the banner this alert would have been if the card still started on `working`.
function startAlert(fleet: Fleet, body: Notification): { title: string; body: string } {
  return { title: fleetHeader(fleet), body: cut(body.subtitle ?? '', ALERT_BODY_BUDGET) }
}

/// Raise a card from the outside, and remember that it is up.
///
/// This is the reason the push-to-start token is stored at all: the agent starts
/// working, or blocks, while the phone is in a pocket, and there is nothing
/// awake on the device to start a card.
///
/// **The alert is not optional and never was.** iOS discards a push-to-start
/// activity that carries no alert dictionary, silently, after APNs has already
/// answered 200 — so the silent `working` start this function used to make was
/// not a quieter card, it was no card at all, and that is why people saw this
/// feature two or three times rather than on every run. The parameter is
/// required here and `sendLiveActivity` refuses a start without one, because a
/// caller that forgot would look from every side like a phone that never got the
/// push.
///
/// A card starts on `blocked` only, and the alert it carries is the fleet's own
/// header. See the end of `pushActivity`, which is where that is decided and
/// argued.
async function startCard(
  env: Env,
  daemon: { account_id: string },
  devices: Device[],
  terminal: string,
  state: ActivityState,
  alert: { title: string; body: string },
): Promise<void> {
  const starters = devices.filter(
    (device): device is Device & { live_activity_start_token: string } =>
      device.platform === 'apns' && !!device.live_activity_start_token,
  )
  // Nothing on the account can raise a card, so nothing was started and there is
  // nothing to remember. Claiming the row here would refuse a card for the rest
  // of the run to a phone that registers its push-to-start token a minute from
  // now.
  if (starters.length === 0) return

  // Push FIRST, and claim the slot only for a start APNs accepted.
  //
  // This was the other way round, and the reasoning for that has expired rather
  // than been overruled. It claimed first because a push that throws is
  // ambiguous, and because forgetting a start that really happened meant the
  // next `working` push ten seconds later started another card, forever. That
  // second half is what made it worth the cost — and `working` does not start
  // cards any more. A card starts on `blocked`, which is rare and is a person
  // waiting, so the stream of retries the pre-claim was defending against no
  // longer exists.
  //
  // What the pre-claim cost, meanwhile, was not hypothetical: a start iOS
  // discarded — every one of them, until this commit, because none carried an
  // alert — still left a row holding this install's only card slot with
  // `update_token = ''`. The relay then believed in a card nobody could see and
  // refused to start another, permanently. `CLAIM_MEMORY_MS` is the second net
  // under that, for the case a push is accepted and still never renders.
  let started = false
  for (const device of starters) {
    const ok = await deliverActivity(
      env,
      daemon.account_id,
      device.live_activity_start_token,
      device.environment,
      {
        event: 'start',
        state,
        alert,
        // Everything the card is ABOUT now travels in the state above, which is
        // the whole point of the restructure: attributes are fixed for an
        // activity's life, so a leader living here could never change. What is
        // left is the card's shape, which genuinely cannot.
        attributes: { version: ACTIVITY_VERSION },
      },
    )
    started = started || ok
  }
  // Nothing was raised, so there is nothing to remember. Claiming the slot for a
  // card that was refused is exactly the bug above.
  if (!started) return

  // The conflict arm updates only a row that is still unaddressable, which is
  // what makes it safe. The app can file the real update token while these
  // pushes are in flight — that is exactly what happens when the phone comes to
  // the foreground because of the alert they carry — and overwriting it with the
  // sentinel would throw away the only address the card has. The `WHERE` is what
  // refuses that, while still letting an escalation record the headline its new
  // card is showing and clear a dismissal it has just superseded.
  //
  // `environment` stays NULL because nothing knows it yet: the start goes to
  // every phone on the account, and whichever one's app runs next reports its
  // own environment alongside the token it files.
  await env.DB.prepare(
    `INSERT INTO install_cards
       (id, account_id, update_token, environment, leader_terminal, leader_status,
        updated_at, pushed_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (account_id)
     DO UPDATE SET leader_terminal = excluded.leader_terminal,
                   leader_status = excluded.leader_status,
                   dismissed_at = NULL,
                   updated_at = excluded.updated_at,
                   pushed_at = excluded.pushed_at
     WHERE install_cards.update_token = ?`,
  )
    .bind(
      crypto.randomUUID(),
      daemon.account_id,
      TOKEN_UNKNOWN,
      null,
      terminal,
      state.status,
      Date.now(),
      Date.now(),
      TOKEN_UNKNOWN,
    )
    .run()
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
): Promise<boolean> {
  const ok = await sendLiveActivity(env, token, activity, environment)
  await record(
    env.METRICS,
    env.ANALYTICS_SALT,
    ok ? 'activity_sent' : 'activity_failed',
    account,
    { platform: 'apns', ok },
  )
  // Whether APNs took it, which `startCard` needs and nothing else does: the
  // card slot is claimed only for a start that was actually accepted.
  return ok
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
