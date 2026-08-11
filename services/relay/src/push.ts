/// Delivering to Apple and Google.
///
/// Both transports live behind one call so that everything above them — and the
/// daemon in particular — never learns which kind of phone it is talking to.
/// The daemon says what happened; where that lands is this file's business.

export interface Payload {
  title: string
  subtitle: string
  /// The terminal to open. Enough to make the notification actionable and
  /// nothing more: no transcript, no command, no output. The relay is a
  /// delivery service and should not be able to leak a conversation it never
  /// held.
  terminal: string
}

/// Which of Apple's two push services issued a device's token.
///
/// A build signed locally carries `aps-environment: development`, so APNs gives
/// it a SANDBOX token, and the production service answers a sandbox token with
/// BadDeviceToken. Posting everything to `api.push.apple.com` therefore meant
/// push was dead for every dev-signed build while looking, from the relay's
/// side, like a device that had simply gone away.
export type Environment = 'development' | 'production'

/// Whether a client named an environment the relay knows.
///
/// Used to reject anything else at the routes rather than fall through to
/// production, because falling through reinstates exactly the bug above and
/// does it silently.
export function isEnvironment(value: unknown): value is Environment {
  return value === 'development' || value === 'production'
}

/// Anything that is not 'development' is production, and that includes NULL:
/// every device registered before the column existed was registered by a build
/// that could only have been talking to the production service.
function apnsHost(environment: string | null | undefined): string {
  return environment === 'development' ? 'api.sandbox.push.apple.com' : 'api.push.apple.com'
}

export async function sendPush(
  env: any,
  platform: string,
  token: string,
  payload: Payload,
  environment: string | null,
): Promise<boolean> {
  return platform === 'apns'
    ? sendApns(env, token, payload, environment)
    : sendFcm(env, token, payload)
}

// MARK: - Apple

async function sendApns(
  env: any,
  token: string,
  payload: Payload,
  environment: string | null,
): Promise<boolean> {
  const jwt = await apnsToken(env)
  const response = await fetch(`https://${apnsHost(environment)}/3/device/${token}`, {
    method: 'POST',
    headers: {
      authorization: `bearer ${jwt}`,
      'apns-topic': env.APNS_TOPIC,
      'apns-push-type': 'alert',
      // Time-sensitive so an agent that is BLOCKED can break a Focus. It has
      // stopped and will stay stopped until answered, which is the definition
      // of the thing this interruption level exists for.
      'apns-priority': '10',
    },
    body: JSON.stringify({
      aps: {
        alert: { title: payload.title, body: payload.subtitle },
        sound: 'default',
        'interruption-level': 'time-sensitive',
        'thread-id': payload.terminal,
      },
      terminal: payload.terminal,
    }),
  })
  return response.ok
}

// MARK: - Live Activities

/// Everything the lock-screen card can say.
///
/// A fixed contract with the iOS app: these are the stored properties of its
/// `ContentState`, by these names. An extra key, or a `status` outside this
/// set, and the app cannot decode the push at all — the activity then freezes
/// on whatever it last showed, which looks exactly like the relay never sent
/// anything.
export interface ActivityState {
  status: 'working' | 'blocked' | 'done'
  detail: string
}

/// What an activity is about, fixed for its whole life.
///
/// Only a `start` carries these. They are the activity's identity rather than
/// its state, so APNs rejects a push that repeats them on an update or an end —
/// which is why the union below makes it impossible to attach them to one.
export interface ActivityAttributes {
  terminal: string
  label: string
  machine: string
}

interface ActivityBase {
  state: ActivityState
  /// What to show if the activity itself cannot be — a locked Apple Watch, a
  /// device that has never run the app.
  alert: { title: string; body: string }
}

export type Activity =
  | (ActivityBase & { event: 'start'; attributes: ActivityAttributes })
  | (ActivityBase & { event: 'update' })
  | (ActivityBase & { event: 'end' })

/// How long the finished state stays up before the card clears itself.
///
/// Without a dismissal date an ended activity sits on the lock screen for up to
/// four hours, which is the state this feature is least useful in and most
/// annoying in. A minute is long enough to be seen by someone picking the phone
/// up because of the alert that went with it.
const DISMISSAL_DELAY_S = 60

/// How long a push-started card may claim to be current.
///
/// There is a real hole under this. When the relay starts a card while the app
/// is not running, nothing reports that activity's update token until the
/// person next opens the app — so the relay has a live card it cannot end. If
/// the agent then finishes, or is answered from the Mac, `done` arrives to find
/// no row and the card sits there saying "Needs You" forever.
///
/// A stale date does not dismiss it. It marks the content as out of date so the
/// system can present it as stale rather than as current truth, which is the
/// honest answer: after an hour of silence the relay genuinely does not know
/// whether this is still true.
///
/// NOT a dismissal date. A card that vanishes on a timer while the agent is
/// still blocked deletes the one notification this whole product exists to
/// deliver, which is a worse failure than showing a stale one.
const STALE_AFTER_S = 60 * 60

/// Push a Live Activity, which is a different message to a different topic than
/// the alert push above even though it goes to the same service.
///
/// The token is NOT the device token: a start uses the app install's
/// push-to-start token and an update or end uses the token APNs issued for that
/// one running activity. Sending either to `/3/device/<device token>` is
/// rejected, and so is the reverse.
export async function sendLiveActivity(
  env: any,
  token: string,
  activity: Activity,
  environment: string | null,
): Promise<boolean> {
  // Seconds. APNs drops an activity push whose timestamp is not newer than the
  // last one it saw for that activity, and `Date.now()` puts the first push
  // fifty-odd thousand years ahead — after which every real update is stale
  // and silently discarded.
  const now = Math.floor(Date.now() / 1000)

  const aps: Record<string, unknown> = {
    timestamp: now,
    event: activity.event,
    'content-state': activity.state,
    alert: activity.alert,
  }
  if (activity.event === 'start') {
    aps['attributes-type'] = 'AgentActivityAttributes'
    aps.attributes = activity.attributes
    aps['stale-date'] = now + STALE_AFTER_S
  }
  if (activity.event === 'end') {
    aps['dismissal-date'] = now + DISMISSAL_DELAY_S
  }

  const response = await fetch(`https://${apnsHost(environment)}/3/device/${token}`, {
    method: 'POST',
    headers: {
      authorization: `bearer ${await apnsToken(env)}`,
      // The alert topic with a suffix, not a topic of its own. APNs routes on
      // it, and the plain bundle id here is a 400 rather than a delivery.
      'apns-topic': `${env.APNS_TOPIC}.push-type.liveactivity`,
      'apns-push-type': 'liveactivity',
      'apns-priority': '10',
    },
    body: JSON.stringify({ aps }),
  })
  return response.ok
}

/// Apple's provider token, cached for its lifetime.
///
/// APNs rejects a token refreshed more often than once every 20 minutes and
/// expires one older than an hour, so this is not merely an optimisation —
/// minting one per notification gets the sender throttled.
let cachedApns: { at: number; jwt: string } | null = null

async function apnsToken(env: any): Promise<string> {
  if (cachedApns && Date.now() - cachedApns.at < 30 * 60_000) return cachedApns.jwt

  const header = { alg: 'ES256', kid: env.APNS_KEY_ID }
  const claims = { iss: env.APNS_TEAM_ID, iat: Math.floor(Date.now() / 1000) }
  const signing = `${base64Url(JSON.stringify(header))}.${base64Url(JSON.stringify(claims))}`

  const key = await crypto.subtle.importKey(
    'pkcs8',
    pemToBytes(env.APNS_KEY_P8),
    { name: 'ECDSA', namedCurve: 'P-256' },
    false,
    ['sign'],
  )
  const signature = await crypto.subtle.sign(
    { name: 'ECDSA', hash: 'SHA-256' },
    key,
    new TextEncoder().encode(signing),
  )
  const jwt = `${signing}.${base64UrlBytes(new Uint8Array(signature))}`
  cachedApns = { at: Date.now(), jwt }
  return jwt
}

// MARK: - Google

async function sendFcm(env: any, token: string, payload: Payload): Promise<boolean> {
  const account = JSON.parse(env.FCM_SERVICE_ACCOUNT)
  const accessToken = await googleAccessToken(account)
  const response = await fetch(
    `https://fcm.googleapis.com/v1/projects/${account.project_id}/messages:send`,
    {
      method: 'POST',
      headers: { authorization: `Bearer ${accessToken}`, 'content-type': 'application/json' },
      body: JSON.stringify({
        message: {
          token,
          notification: { title: payload.title, body: payload.subtitle },
          android: { priority: 'HIGH' },
          data: { terminal: payload.terminal },
        },
      }),
    },
  )
  return response.ok
}

let cachedGoogle: { at: number; token: string } | null = null

async function googleAccessToken(account: any): Promise<string> {
  if (cachedGoogle && Date.now() - cachedGoogle.at < 30 * 60_000) return cachedGoogle.token

  const now = Math.floor(Date.now() / 1000)
  const header = { alg: 'RS256', typ: 'JWT' }
  const claims = {
    iss: account.client_email,
    scope: 'https://www.googleapis.com/auth/firebase.messaging',
    aud: 'https://oauth2.googleapis.com/token',
    iat: now,
    exp: now + 3600,
  }
  const signing = `${base64Url(JSON.stringify(header))}.${base64Url(JSON.stringify(claims))}`
  const key = await crypto.subtle.importKey(
    'pkcs8',
    pemToBytes(account.private_key),
    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
    false,
    ['sign'],
  )
  const signature = await crypto.subtle.sign(
    'RSASSA-PKCS1-v1_5',
    key,
    new TextEncoder().encode(signing),
  )
  const assertion = `${signing}.${base64UrlBytes(new Uint8Array(signature))}`

  const response = await fetch('https://oauth2.googleapis.com/token', {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'urn:ietf:params:oauth:grant-type:jwt-bearer',
      assertion,
    }),
  })
  const body = await response.json<{ access_token: string }>()
  cachedGoogle = { at: Date.now(), token: body.access_token }
  return body.access_token
}

// MARK: - Encoding

function base64Url(text: string): string {
  return base64UrlBytes(new TextEncoder().encode(text))
}

function base64UrlBytes(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replaceAll('=', '')
}

/// A PEM's body, decoded. Both Apple's `.p8` and Google's service-account key
/// arrive this way.
function pemToBytes(pem: string): Uint8Array {
  const body = pem
    .replace(/-----[A-Z ]+-----/g, '')
    .replaceAll('\n', '')
    .replaceAll('\r', '')
    .trim()
  return Uint8Array.from(atob(body), c => c.charCodeAt(0))
}
