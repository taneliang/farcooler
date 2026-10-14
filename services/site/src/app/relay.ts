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
}

/// Whether both halves of the configuration are there.
export function configured(env: AppEnv): env is Required<AppEnv> {
  return Boolean(env.APP_RELAY_URL?.trim() && env.APP_WORKOS_CLIENT_ID?.trim())
}

async function post(env: Required<AppEnv>, path: string, body: unknown, token?: string): Promise<Response> {
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
export async function exchange(env: Required<AppEnv>, code: string, verifier: string): Promise<Tokens | null> {
  return await tokensFrom(await post(env, '/v1/auth/token', { code, verifier }))
}

/// What a directory call came to.
export type Answer =
  | { kind: 'ok'; body: any }
  | { kind: 'refused'; status: number }
  | { kind: 'signed-out' }
  | { kind: 'unreachable' }

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
    private readonly env: Required<AppEnv>,
    public tokens: Tokens,
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
    } catch {
      // A network failure. Nothing is logged: the error can carry the request.
      return { kind: 'unreachable' }
    }
  }

  private async refresh(): Promise<boolean> {
    this.refreshed = true
    const response = await post(this.env, '/v1/auth/refresh', { refreshToken: this.tokens.refresh })
    if (response.status >= 500 || response.status === 429) throw new Error('relay unavailable')
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
