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

export async function sendPush(
  env: any,
  platform: string,
  token: string,
  payload: Payload,
): Promise<boolean> {
  return platform === 'apns' ? sendApns(env, token, payload) : sendFcm(env, token, payload)
}

// MARK: - Apple

async function sendApns(env: any, token: string, payload: Payload): Promise<boolean> {
  const jwt = await apnsToken(env)
  const response = await fetch(`https://api.push.apple.com/3/device/${token}`, {
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
