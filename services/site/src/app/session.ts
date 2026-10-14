/// The web app's session: WorkOS's own tokens, kept in an HttpOnly cookie.
///
/// The cookie holds the access and refresh tokens WorkOS issued at sign-in,
/// exactly as the apps hold them in the Keychain. Nothing here mints a session
/// of its own (ov-480 design, "Least privilege"): the relay verifies the access
/// token on every directory call, and WorkOS decides whether the refresh token
/// still works. The Worker only carries the tokens between the browser and the
/// relay, and never logs or echoes them.
///
/// Not sealed with a key of our own. The access token is signed by WorkOS and
/// checked by the relay, and the refresh token is checked by WorkOS, so a
/// forged cookie gets nothing; a stolen one would be replayable sealed or not.
/// HttpOnly keeps page script away from it, `Secure` and the `__Secure-`
/// prefix keep it off plain HTTP, and `Path=/app` keeps it off the marketing
/// page's requests.

export const SESSION_COOKIE = '__Secure-fc_session'
/// The sign-in in flight: its state and PKCE verifier, for the callback only.
export const SIGN_IN_COOKIE = '__Secure-fc_signin'

/// Thirty days, WorkOS's default session length. The refresh token stops
/// working when WorkOS ends the session, whatever the cookie says.
const SESSION_MAX_AGE = 30 * 24 * 60 * 60
/// Ten minutes to finish signing in.
const SIGN_IN_MAX_AGE = 10 * 60

export interface Tokens {
  access: string
  refresh: string
}

export interface SignIn {
  state: string
  verifier: string
}

/// The value of one cookie on the request, or null.
export function cookie(request: Request, name: string): string | null {
  const header = request.headers.get('cookie') ?? ''
  for (const part of header.split(';')) {
    const at = part.indexOf('=')
    if (at < 0) continue
    if (part.slice(0, at).trim() === name) return part.slice(at + 1).trim()
  }
  return null
}

/// The session on the request, or null for none or one that doesn't parse.
export function readSession(request: Request): Tokens | null {
  const value = decode(cookie(request, SESSION_COOKIE))
  if (typeof value?.a !== 'string' || typeof value?.r !== 'string' || !value.a || !value.r) return null
  return { access: value.a, refresh: value.r }
}

export function readSignIn(request: Request): SignIn | null {
  const value = decode(cookie(request, SIGN_IN_COOKIE))
  if (typeof value?.s !== 'string' || typeof value?.v !== 'string' || !value.s || !value.v) return null
  return { state: value.s, verifier: value.v }
}

export function sessionCookie(tokens: Tokens): string {
  return `${SESSION_COOKIE}=${encode({ a: tokens.access, r: tokens.refresh })}; Path=/app; HttpOnly; Secure; SameSite=Lax; Max-Age=${SESSION_MAX_AGE}`
}

/// Path=/app/callback: the only request that needs it.
export function signInCookie(signIn: SignIn): string {
  return `${SIGN_IN_COOKIE}=${encode({ s: signIn.state, v: signIn.verifier })}; Path=/app/callback; HttpOnly; Secure; SameSite=Lax; Max-Age=${SIGN_IN_MAX_AGE}`
}

export function clearSession(): string {
  return `${SESSION_COOKIE}=; Path=/app; HttpOnly; Secure; SameSite=Lax; Max-Age=0`
}

export function clearSignIn(): string {
  return `${SIGN_IN_COOKIE}=; Path=/app/callback; HttpOnly; Secure; SameSite=Lax; Max-Age=0`
}

/// A fresh state and PKCE pair, and the challenge to send WorkOS.
export async function newSignIn(): Promise<SignIn & { challenge: string }> {
  const verifier = randomToken()
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier))
  return { state: randomToken(), verifier, challenge: base64Url(new Uint8Array(digest)) }
}

/// Whether the access token expires within a minute, read without verifying
/// it. Only to refresh before the relay would refuse it; the relay is what
/// checks the signature. A token that can't be read is sent as it is.
export function expiresSoon(access: string, now = Date.now()): boolean {
  const exp = claims(access)?.exp
  return typeof exp === 'number' && exp * 1000 < now + 60_000
}

/// The WorkOS session id, which signing out ends at WorkOS too.
export function sessionId(access: string): string | null {
  const sid = claims(access)?.sid
  return typeof sid === 'string' && /^[A-Za-z0-9_]{1,128}$/.test(sid) ? sid : null
}

function claims(token: string): Record<string, unknown> | null {
  const parts = token.split('.')
  if (parts.length !== 3) return null
  const value = decode(parts[1])
  return value && typeof value === 'object' ? value : null
}

function randomToken(): string {
  return base64Url(crypto.getRandomValues(new Uint8Array(32)))
}

function encode(value: unknown): string {
  return base64Url(new TextEncoder().encode(JSON.stringify(value)))
}

function decode(text: string | null): Record<string, any> | null {
  if (!text || !/^[A-Za-z0-9_-]+$/.test(text)) return null
  try {
    const binary = atob(text.replaceAll('-', '+').replaceAll('_', '/').padEnd(Math.ceil(text.length / 4) * 4, '='))
    const value = JSON.parse(new TextDecoder().decode(Uint8Array.from(binary, c => c.charCodeAt(0))))
    return value && typeof value === 'object' && !Array.isArray(value) ? value : null
  } catch {
    return null
  }
}

function base64Url(bytes: Uint8Array): string {
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
}
