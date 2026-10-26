/// The web client's Worker (ov-764): app.farcooler.com and
/// app.canary.farcooler.com.
///
/// It signs the visitor in with WorkOS AuthKit exactly as the site's /app does
/// (the shared code is in services/common), and hands the client shell only to
/// an account the flag admits (flag.ts). Everyone else gets a page with no
/// script. Paths, identical on both hosts (plan-763 §1):
///
/// - `GET /`: the shell, or sign-in, or "Not available yet".
/// - `/sign-in`, `/callback`, `POST /sign-out`: AuthKit, with PKCE.
/// - `POST /api/relay/<route>`: api.ts.
/// - `/a/<name>.<hash>.<ext>` and `/version.json`: assets.ts.
/// - `POST /csp`: csp-report.ts.

import { sameOrigin } from '../../common/origin'
import { Caller, configured, exchange, visitorOf, withinLimit, type Configured } from '../../common/relay'
import { clearSession, clearSignIn, cookie, newCsrf, newSignIn, readSession, readSignIn, SESSION_COOKIE, sessionCookie, sessionId, signInCookie } from '../../common/session'
import { api } from './api'
import { asset, isAsset, version } from './assets'
import { cspReport } from './csp-report'
import type { WebEnv } from './env'
import { admitted, open } from './flag'
import { page, redirect, shellCsp, withHeaders } from './headers'
import { limitedPage, notAvailablePage, notFoundPage, signInFailedPage, signInPage, unreachablePage } from './pages'

export type { WebEnv } from './env'

export default {
  async fetch(request: Request, env: WebEnv): Promise<Response> {
    const url = new URL(request.url)
    // One host only. workers_dev and preview_urls are off in wrangler.toml;
    // this keeps a second origin from serving the client if either comes back.
    const host = env.APP_HOST?.trim()
    if (!host || url.hostname !== host || url.protocol !== 'https:') return new Response(null, { status: 404 })
    return await route(request, url, env)
  },
} satisfies ExportedHandler<WebEnv>

async function route(request: Request, url: URL, env: WebEnv): Promise<Response> {
  const path = url.pathname
  const method = request.method

  if (path.startsWith('/a/')) {
    if (method !== 'GET' && method !== 'HEAD') return withHeaders(null, 405, { allow: 'GET, HEAD' })
    return (isAsset(path) && (await asset(env.ASSETS, url, method))) || page(notFoundPage(), 404)
  }
  if (path === '/version.json') return await version(env.ASSETS, url)
  if (path === '/csp') return method === 'POST' ? await cspReport(request) : withHeaders(null, 405, { allow: 'POST' })
  if (path === '/sign-out') return method === 'POST' ? signOut(request, url) : withHeaders(null, 405, { allow: 'POST' })

  // Everything below is the client or its way in. With the flag off, or the
  // channel not set up, there is no sign-in and no API: only this page.
  if (!configured(env) || !open(env)) {
    if (path.startsWith('/api/')) return withHeaders(null, 404)
    return page(notAvailablePage(false), path === '/' ? 200 : 404)
  }
  if (path.startsWith('/api/relay/')) return await api(request, url, env)
  if (method !== 'GET' && method !== 'HEAD') return withHeaders(null, 405, { allow: 'GET, HEAD' })
  if (path === '/sign-in') return await startSignIn(url, env)
  if (path === '/callback') return await finishSignIn(request, url, env)
  if (path === '/') return await home(request, url, env)
  return page(notFoundPage(), 404)
}

/// The shell for an admitted account; sign-in when signed out; otherwise
/// "Not available yet".
async function home(request: Request, url: URL, env: Configured & WebEnv): Promise<Response> {
  const tokens = readSession(request)
  if (!tokens) {
    const stale = cookie(request, SESSION_COOKIE) === null ? [] : [clearSession()]
    return page(signInPage(), 200, {}, stale)
  }
  const caller = new Caller(env, tokens, visitorOf(request))
  const me = await caller.directory('me')
  if (caller.signedOut || me.kind === 'signed-out') return page(signInPage(), 200, {}, [clearSession()])
  const cookies = caller.renewed ? [sessionCookie(caller.tokens)] : []
  if (me.kind === 'limited') return page(limitedPage(), 429, { 'retry-after': '60' }, cookies)
  if (me.kind !== 'ok') return page(unreachablePage(), 502, {}, cookies)
  if (!admitted(env, me.body?.user?.id)) return page(notAvailablePage(true), 200, {}, cookies)

  const shell = await env.ASSETS.fetch(new URL('/index.html', url.origin))
  if (shell.status !== 200) return page(unreachablePage(), 503, {}, cookies)
  return withHeaders(
    shell.body,
    200,
    {
      'cache-control': 'no-store',
      'content-type': 'text/html; charset=utf-8',
      'content-security-policy': shellCsp(env),
      'reporting-endpoints': 'csp="/csp"',
      'x-frame-options': 'DENY',
    },
    cookies,
  )
}

/// Off to AuthKit, with PKCE and a state bound to this browser.
async function startSignIn(url: URL, env: Configured): Promise<Response> {
  const signIn = await newSignIn()
  const authorize = new URL('https://api.workos.com/user_management/authorize')
  authorize.search = new URLSearchParams({
    client_id: env.APP_WORKOS_CLIENT_ID.trim(),
    redirect_uri: `${url.origin}/callback`,
    response_type: 'code',
    provider: 'authkit',
    code_challenge: signIn.challenge,
    code_challenge_method: 'S256',
    state: signIn.state,
  }).toString()
  return redirect(authorize.toString(), [signInCookie(signIn)])
}

/// Back from AuthKit: check the state, trade the code through the relay, and
/// keep the session.
async function finishSignIn(request: Request, url: URL, env: Configured): Promise<Response> {
  const pending = readSignIn(request)
  const code = url.searchParams.get('code')
  const state = url.searchParams.get('state')
  const failed = () => page(signInFailedPage(), 400, {}, [clearSignIn()])
  if (!pending || !code || !state || state !== pending.state) return failed()
  if (!(await withinLimit(env, visitorOf(request)))) return page(limitedPage(), 429, { 'retry-after': '60' }, [clearSignIn()])
  let tokens
  try {
    tokens = await exchange(env, code, pending.verifier, visitorOf(request))
  } catch {
    return page(unreachablePage(), 502, {}, [clearSignIn()])
  }
  if (tokens === 'limited') return page(limitedPage(), 429, { 'retry-after': '60' }, [clearSignIn()])
  if (!tokens) return failed()
  return redirect('/', [clearSignIn(), sessionCookie({ ...tokens, csrf: newCsrf() })])
}

/// Forget the session here, then end it at WorkOS too, so the next Sign In
/// asks again. A post from another site is refused before anything is cleared.
///
/// It also clears this origin's storage (`Clear-Site-Data: "storage"`): the
/// browser's SSH key and its runners belong to the account that signed out, and
/// the next person to sign in on this browser must not inherit them. The page
/// deletes them itself before it posts here; this covers a sign-out the page
/// didn't run (ov-764 review).
function signOut(request: Request, url: URL): Response {
  if (!sameOrigin(request, url)) return withHeaders(null, 403)
  const cleared = [clearSession(), clearSignIn()]
  const tokens = readSession(request)
  const sid = tokens ? sessionId(tokens.access) : null
  let to = '/'
  if (sid) {
    const logout = new URL('https://api.workos.com/user_management/sessions/logout')
    logout.search = new URLSearchParams({ session_id: sid, return_to: `${url.origin}/` }).toString()
    to = logout.toString()
  }
  return withHeaders(null, 303, { 'cache-control': 'no-store', location: to, 'clear-site-data': '"storage"' }, cleared)
}
