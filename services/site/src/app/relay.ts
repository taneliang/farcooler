/// Calls to the relay, from the Worker, never from the browser.
///
/// Server to server, so the relay needs no CORS and the browser never holds a
/// token page script could read. Sign-in goes through the relay's own
/// `/v1/auth/token` and `/v1/auth/refresh`, the routes the apps use: the relay
/// has the WorkOS API key and creates the account, and this Worker needs no
/// secret at all.

import { expiresSoon, type Tokens } from './session'

export interface AppEnv {
  /// The relay the web app reads, e.g. https://relay.farcooler.com. Empty
  /// means the web app isn't set up yet.
  APP_RELAY_URL?: string
  /// That relay's WorkOS client id (the `WORKOS_CLIENT_ID` in its
  /// wrangler.toml block). Empty means the web app isn't set up yet.
  APP_WORKOS_CLIENT_ID?: string
  /// Per visitor, before every /v1/auth call (wrangler.toml). The relay's own
  /// limit on those routes is one bucket for this whole Worker, since its
  /// calls all come from Cloudflare; this keeps one visitor from emptying it.
  APP_AUTH_LIMIT?: RateLimit
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

/// The visitor's address, as Cloudflare reports it.
export function visitorOf(request: Request): string {
  return request.headers.get('cf-connecting-ip') ?? 'unknown'
}

class Limited extends Error {}

async function post(env: Configured, path: string, body: unknown, token?: string): Promise<Response> {
  const headers: Record<string, string> = { 'content-type': 'application/json' }
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
export async function exchange(env: Configured, code: string, verifier: string): Promise<Tokens | null | 'limited'> {
  const response = await post(env, '/v1/auth/token', { code, verifier })
  return response.status === 429 ? 'limited' : await tokensFrom(response)
}

/// What a directory call came to.
export type Answer =
  | { kind: 'ok'; body: any }
  | { kind: 'refused'; status: number }
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
    if (this.signedOut) return { kind: 'signed-out' }
    try {
      if (!this.refreshed && expiresSoon(this.tokens.access) && !(await this.refresh())) return { kind: 'signed-out' }
      let response = await post(this.env, `/v1/directory/${route}`, body, this.tokens.access)
      if (response.status === 401 && !this.refreshed) {
        if (!(await this.refresh())) return { kind: 'signed-out' }
        response = await post(this.env, `/v1/directory/${route}`, body, this.tokens.access)
      }
      if (response.status === 401) {
        this.signedOut = true
        return { kind: 'signed-out' }
      }
      if (response.status >= 500) return { kind: 'unreachable' }
      if (!response.ok) return { kind: 'refused', status: response.status }
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
    const response = await post(this.env, '/v1/auth/refresh', { refreshToken: this.tokens.refresh })
    if (response.status === 429) throw new Limited()
    if (response.status >= 500) throw new Error('relay unavailable')
    const tokens = await tokensFrom(response)
    if (!tokens) {
      this.signedOut = true
      return false
    }
    this.tokens = tokens
    this.renewed = true
    return true
  }
}
