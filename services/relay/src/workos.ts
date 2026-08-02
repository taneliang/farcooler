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

export interface Session {
  userId: string
  email?: string
}

/// Verify an access token and return who it belongs to.
///
/// Against the JWKS rather than by calling WorkOS on every request: a
/// notification path that needs a round trip to a third party before it can
/// deliver anything is a notification path that is down whenever they are.
export async function verifySession(
  token: string,
  env: { WORKOS_CLIENT_ID: string },
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
  if (!claims?.sub) return null
  // Expiry is checked here rather than trusted from the caller: a token that
  // verifies is not the same as a token that is still good.
  if (typeof claims.exp === 'number' && claims.exp * 1000 < Date.now()) return null

  return { userId: claims.sub, email: claims.email }
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
