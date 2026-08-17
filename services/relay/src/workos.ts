/// Identity, which is WorkOS's job rather than this service's.
///
/// The relay verifies a session and learns a user id. It never sees a password,
/// never runs an email flow, never issues a session of its own — so there is no
/// second identity system to keep in step with the first, and no credential
/// store here to breach.
///
/// WorkOS rather than Sign in with Apple alone because Android is coming: an
/// Apple-only identity would have to be replaced rather than extended, and
/// identity migrations are the worst kind. It also means the day a company asks
/// for Okta or SCIM, that is configuration rather than a rewrite.

/// Who a verified token belongs to — and deliberately not WHEN they last
/// proved it.
///
/// A WorkOS access token carries no `auth_time`: the claim list is `iss`, `sub`,
/// `client_id`, `act`, `org_id`, `role`, `roles`, `permissions`,
/// `entitlements`, `feature_flags`, `sid`, `jti`, `exp`, `iat`. So token-based
/// freshness is not available here at all, and `iat` cannot stand in for it —
/// this relay mints fresh access tokens from refresh tokens at
/// `/v1/auth/refresh`, so a recent `iat` proves a refresh happened, not that a
/// human authenticated.
///
/// The onboarding confirmation's freshness therefore comes from
/// LocalAuthentication on the device — biometry or a passcode at the moment of
/// the tap — which the design already names as the gate that actually matters.
/// That is a stronger answer than a claim would be: it is about the person
/// holding the phone now, not about a session that began at some point.
export interface Session {
  userId: string
  email?: string
}

/// A minute, to absorb the clock skew between WorkOS's machines and this one.
///
/// Small on purpose: it is the amount by which an expired token still works,
/// and every second of it is bought from the wrong side of the trade.
const LEEWAY_SECONDS = 60

/// Verify an access token and return who it belongs to.
///
/// Against the JWKS rather than by calling WorkOS on every request: a
/// notification path that needs a round trip to a third party before it can
/// deliver anything is a notification path that is down whenever they are.
///
/// Every claim the authorization depends on is checked, which it once was not.
/// A valid signature says WorkOS minted this token — not that it was minted for
/// THIS application, and not that it is still good. Two applications in one
/// WorkOS environment share a key set, so the other one's token verified here
/// and authenticated as its `sub`, against an account it had nothing to do
/// with; and expiry was checked only when `exp` happened to be present, so a
/// token without one was a session that never ended.
export async function verifySession(
  token: string,
  env: { WORKOS_CLIENT_ID: string; WORKOS_ISSUER: string },
): Promise<Session | null> {
  const parts = token.split('.')
  if (parts.length !== 3) return null

  const keys = await jwks(env.WORKOS_CLIENT_ID)
  const header = decodeSegment(parts[0])
  const key = keys.find(k => k.kid === header?.kid)
  if (!key) return null

  const material = await crypto.subtle.importKey(
    'jwk',
    key,
    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
    false,
    ['verify'],
  )
  const signed = new TextEncoder().encode(`${parts[0]}.${parts[1]}`)
  const signature = base64UrlToBytes(parts[2])
  if (!(await crypto.subtle.verify('RSASSA-PKCS1-v1_5', material, signature, signed))) {
    return null
  }

  const claims = decodeSegment(parts[1])
  if (!claims) return null
  const now = Math.floor(Date.now() / 1000)

  // Required, not checked-if-present. Reading absence as "nothing to object
  // to" enforces a claim only on the tokens that volunteered it, which is every
  // token except the one worth worrying about.
  if (typeof claims.exp !== 'number' || claims.exp + LEEWAY_SECONDS < now) return null
  if (typeof claims.iat !== 'number' || claims.iat - LEEWAY_SECONDS > now) return null
  if (typeof claims.nbf === 'number' && claims.nbf - LEEWAY_SECONDS > now) return null
  if (claims.iss !== env.WORKOS_ISSUER) return null
  if (!boundToThisApplication(claims, env.WORKOS_CLIENT_ID)) return null
  if (typeof claims.sub !== 'string' || claims.sub.length === 0) return null

  return { userId: claims.sub, email: claims.email }
}

/// That this token was minted for THIS application, not the one next door.
///
/// The check is on `client_id`, because a WorkOS access token has no `aud` — its
/// claims are `iss`, `sub`, `client_id`, `act`, `org_id`, `role`, `roles`,
/// `permissions`, `entitlements`, `feature_flags`, `sid`, `jti`, `exp`, `iat`.
/// Requiring the audience an OIDC token would carry would refuse every real
/// token; `client_id` is the claim that exists and it answers the same
/// question, which is the one that matters: two applications in one WorkOS
/// environment share a key set, so without this the other one's token
/// authenticates here as its `sub`.
///
/// `aud` is honoured when it happens to be there — a custom AuthKit domain or a
/// later token format may start sending it — but its absence is not a failure,
/// only `client_id`'s. `aud` is a string or an array of them, per RFC 7519, and
/// comparing the raw claim would refuse every token the day the array form
/// appeared.
function boundToThisApplication(claims: Record<string, any>, clientId: string): boolean {
  if (typeof claims.client_id === 'string') return claims.client_id === clientId
  return audienceMatches(claims.aud, clientId)
}

function audienceMatches(aud: unknown, clientId: string): boolean {
  if (typeof aud === 'string') return aud === clientId
  if (Array.isArray(aud)) return aud.includes(clientId)
  return false
}

/// Cached for an hour. Key rotation is rare and a fetch per notification would
/// put a third party on the critical path of every one.
let cachedKeys: { at: number; keys: JsonWebKey[] } | null = null

async function jwks(clientId: string): Promise<(JsonWebKey & { kid?: string })[]> {
  if (cachedKeys && Date.now() - cachedKeys.at < 3_600_000) return cachedKeys.keys
  const response = await fetch(`https://api.workos.com/sso/jwks/${clientId}`)
  const body = await response.json<{ keys: JsonWebKey[] }>()
  cachedKeys = { at: Date.now(), keys: body.keys ?? [] }
  return cachedKeys.keys
}

function decodeSegment(segment: string): Record<string, any> | null {
  try {
    return JSON.parse(new TextDecoder().decode(base64UrlToBytes(segment)))
  } catch {
    return null
  }
}

function base64UrlToBytes(text: string): Uint8Array {
  const padded = text.replaceAll('-', '+').replaceAll('_', '/')
  const binary = atob(padded.padEnd(Math.ceil(padded.length / 4) * 4, '='))
  return Uint8Array.from(binary, c => c.charCodeAt(0))
}
