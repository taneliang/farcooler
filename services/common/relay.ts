/// Calls to the relay, from the Worker, never from the browser.
///
/// Server to server, so the relay needs no CORS and the browser never holds a
/// token page script could read. Sign-in goes through the relay's own
/// `/v1/auth/token` and `/v1/auth/refresh`, the routes the apps use: the relay
/// has the WorkOS API key and creates the account, and this Worker needs no
/// secret at all. Shared by the site's /app and the web client (ov-764).

import { expiresSoon, type Tokens } from './session'
import { VISITOR_CONTEXT } from './visitor'

export interface AppEnv {
  /// The relay the web app reads, e.g. https://relay.farcooler.com. Empty
  /// means the web app isn't set up yet.
  APP_RELAY_URL?: string
  /// That relay's WorkOS client id (the `WORKOS_CLIENT_ID` in its
  /// wrangler.toml block), the one the apps use too. Empty means the web app
  /// isn't set up yet.
  APP_WORKOS_CLIENT_ID?: string
  /// Per visitor, before every /v1/auth call (wrangler.toml). The relay's own
  /// limit on those routes is keyed on the caller's address, and this Worker's
  /// calls all come from Cloudflare's; this keeps one visitor from emptying
  /// that bucket when the relay can't tell visitors apart (no `VISITOR_SECRET`).
  APP_AUTH_LIMIT?: RateLimit
  /// A secret shared with the relay (its `VISITOR_SECRET`, the same value). With
  /// it, each /v1/auth call names the visitor and signs the name, and the relay
  /// limits that visitor rather than this Worker as a whole (ov-811). Without
  /// it, or with a different value, the relay keys on the address as before.
  VISITOR_SECRET?: string
}

export type Configured = Required<Pick<AppEnv, 'APP_RELAY_URL' | 'APP_WORKOS_CLIENT_ID'>> & AppEnv

/// Whether both halves of the configuration are there, and the relay is
/// reached over https: a token never travels in the clear.
export function configured(env: AppEnv): env is Configured {
  if (!env.APP_RELAY_URL?.trim() || !env.APP_WORKOS_CLIENT_ID?.trim()) return false
  try {
    return new URL(env.APP_RELAY_URL.trim()).protocol === 'https:'
  } catch {
    return false
  }
}

/// Whether this visitor may make another /v1/auth call now. Keyed on their
/// own address. Open when the binding is absent (a local `wrangler dev`), as
/// the relay's limit is.
export async function withinLimit(env: AppEnv, visitor: string): Promise<boolean> {
  if (!env.APP_AUTH_LIMIT) return true
  const { success } = await env.APP_AUTH_LIMIT.limit({ key: visitor })
  return success
}

/// Who the visitor is, for a rate limit: their address as Cloudflare reports
/// it, or for IPv6 the /64 it sits in (`2001:db8:1:2::/64`). One subscriber is
/// normally given a whole /64 and can use any address in it, so keying on the
/// full address would give one host 2^64 buckets (ov-811).
export function visitorOf(request: Request): string {
  const address = request.headers.get('cf-connecting-ip')?.trim()
  if (!address) return 'unknown'
  return address.includes(':') ? (prefix64(address) ?? address) : address
}

/// The /64 an IPv6 address is in, written in full lowercase hex groups, or null
/// when it isn't one. An IPv4-mapped address (`::ffff:203.0.113.7`) is the
/// IPv4 address itself.
export function prefix64(address: string): string | null {
  const mapped = /^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/i.exec(address)
  if (mapped) return mapped[1]!
  const halves = address.toLowerCase().split('::')
  if (halves.length > 2) return null
  const groups = (part: string) => (part === '' ? [] : part.split(':'))
  const head = groups(halves[0]!)
  const tail = halves.length === 2 ? groups(halves[1]!) : []
  const missing = 8 - head.length - tail.length
  if (halves.length === 2 ? missing < 1 : missing !== 0) return null
  const all = [...head, ...Array<string>(missing).fill('0'), ...tail]
  if (!all.every(group => /^[0-9a-f]{1,4}$/.test(group))) return null
  return `${all.slice(0, 4).map(group => group.padStart(4, '0')).join(':')}::/64`
}

/// The headers that name the visitor to the relay, signed with the shared
/// secret: `x-fc-visitor` and `x-fc-visitor-mac`, an HMAC-SHA256 of
/// `VISITOR_CONTEXT` and the name. Nothing when there's no secret or visitor.
/// The relay checks them in `visitorKey` (services/relay/src/index.ts).
export async function visitorHeaders(env: AppEnv, visitor?: string): Promise<Record<string, string>> {
  const secret = env.VISITOR_SECRET?.trim()
  if (!secret || !visitor) return {}
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign'])
  const mac = await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(`${VISITOR_CONTEXT}${visitor}`))
  const hex = [...new Uint8Array(mac)].map(byte => byte.toString(16).padStart(2, '0')).join('')
  return { 'x-fc-visitor': visitor, 'x-fc-visitor-mac': hex }
}


class Limited extends Error {}

async function post(env: Configured, path: string, body: unknown, token?: string, visitor?: string): Promise<Response> {
  const headers: Record<string, string> = { 'content-type': 'application/json', ...(await visitorHeaders(env, visitor)) }
  if (token) headers.authorization = `Bearer ${token}`
  return await fetch(`${env.APP_RELAY_URL.trim().replace(/\/+$/, '')}${path}`, {
    method: 'POST',
    headers,
    body: JSON.stringify(body),
  })
}

/// The relay's answer to a sign-in or refresh, or null for a refusal.
async function tokensFrom(response: Response): Promise<Tokens | null> {
  if (!response.ok) return null
  const body = await response.json<{ accessToken?: unknown; refreshToken?: unknown }>().catch(() => null)
  if (typeof body?.accessToken !== 'string' || typeof body?.refreshToken !== 'string') return null
  if (!body.accessToken || !body.refreshToken) return null
  return { access: body.accessToken, refresh: body.refreshToken }
}

/// Trade the code from WorkOS for a session, proving the PKCE verifier.
/// 'limited' when the relay's own limit refused the call.
export async function exchange(env: Configured, code: string, verifier: string, visitor: string): Promise<Tokens | null | 'limited'> {
  const response = await post(env, '/v1/auth/token', { code, verifier }, undefined, visitor)
  return response.status === 429 ? 'limited' : await tokensFrom(response)
}

/// What a directory call came to.
export type Answer =
  | { kind: 'ok'; body: any }
  | { kind: 'refused'; status: number; error?: string; reason?: string }
  | { kind: 'signed-out' }
  | { kind: 'unreachable' }
  | { kind: 'limited' }

/// One signed-in visitor's calls to the directory, refreshing the access token
/// at most once per request, before it expires or when the relay refuses it.
///
/// `tokens` is what the response must set when `renewed` is true; when
/// `signedOut` is true the session is over and the cookie must go.
export class Caller {
  renewed = false
  signedOut = false
  private refreshed = false

  constructor(
    private readonly env: Configured,
    public tokens: Tokens,
    private readonly visitor: string,
  ) {}

  async directory(route: string, body: Record<string, unknown> = {}): Promise<Answer> {
    return await this.call(`/v1/directory/${route}`, body)
  }

  /// A call to the relay's account routes (`/v1/account/...`), answered the
  /// same way. Sign Out Everywhere is one (ov-564).
  async account(route: string, body: Record<string, unknown> = {}): Promise<Answer> {
    return await this.call(`/v1/account/${route}`, body)
  }

  private async call(path: string, body: Record<string, unknown>): Promise<Answer> {
    if (this.signedOut) return { kind: 'signed-out' }
    try {
      if (!this.refreshed && expiresSoon(this.tokens.access) && !(await this.refresh())) return { kind: 'signed-out' }
      let response = await post(this.env, path, body, this.tokens.access)
      if (response.status === 401 && !this.refreshed) {
        if (!(await this.refresh())) return { kind: 'signed-out' }
        response = await post(this.env, path, body, this.tokens.access)
      }
      if (response.status === 401) {
        this.signedOut = true
        return { kind: 'signed-out' }
      }
      if (response.status >= 500) return { kind: 'unreachable' }
      // The relay's per-account limit on directory/me (ov-815), or any other
      // route's: "wait a minute", not "unreachable".
      if (response.status === 429) return { kind: 'limited' }
      if (!response.ok) {
        const said = await response.json<{ error?: unknown; reason?: unknown }>().catch(() => null)
        return {
          kind: 'refused',
          status: response.status,
          ...(typeof said?.error === 'string' ? { error: said.error } : {}),
          ...(typeof said?.reason === 'string' ? { reason: said.reason } : {}),
        }
      }
      const parsed = await response.json().catch(() => undefined)
      return parsed === undefined ? { kind: 'unreachable' } : { kind: 'ok', body: parsed }
    } catch (error) {
      // Nothing is logged: a network error can carry the request.
      return error instanceof Limited ? { kind: 'limited' } : { kind: 'unreachable' }
    }
  }

  private async refresh(): Promise<boolean> {
    this.refreshed = true
    if (!(await withinLimit(this.env, this.visitor))) throw new Limited()
    const response = await post(this.env, '/v1/auth/refresh', { refreshToken: this.tokens.refresh }, undefined, this.visitor)
    if (response.status === 429) throw new Limited()
    if (response.status >= 500) throw new Error('relay unavailable')
    const tokens = await tokensFrom(response)
    if (!tokens) {
      this.signedOut = true
      return false
    }
    this.tokens = { ...tokens, ...(this.tokens.csrf ? { csrf: this.tokens.csrf } : {}) }
    this.renewed = true
    return true
  }
}
