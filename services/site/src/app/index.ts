/// The web app at www.farcooler.com/app (ov-517, team directory phase 3).
///
/// Server-rendered: the Worker signs the visitor in with WorkOS AuthKit, keeps
/// WorkOS's tokens in an HttpOnly cookie (`session.ts`), reads the relay's
/// directory API with them (`relay.ts`), and returns plain HTML (`pages.ts`).
/// The browser never runs our script and never holds a token it can read.
///
/// Signed out, every page is the sign-in page: no route here reads the
/// directory without a session, and the relay checks that session again.

import {
  forbiddenPage,
  limitedPage,
  membersPage,
  notConfiguredPage,
  notFoundPage,
  signInFailedPage,
  signInPage,
  teamsPage,
  unreachablePage,
  type Member,
  type Team,
  type Viewer,
} from './pages'
import { Caller, configured, exchange, visitorOf, withinLimit, type Answer, type AppEnv, type Configured } from './relay'
import {
  clearSession,
  clearSignIn,
  cookie,
  newSignIn,
  readSession,
  readSignIn,
  SESSION_COOKIE,
  sessionCookie,
  sessionId,
  signInCookie,
} from './session'

export type { AppEnv } from './relay'

/// Whether a path belongs to the web app.
export function isApp(pathname: string): boolean {
  return pathname === '/app' || pathname.startsWith('/app/')
}

const SECURITY_HEADERS: Record<string, string> = {
  'cache-control': 'no-store',
  'content-security-policy':
    "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; form-action 'self' https://api.workos.com; frame-ancestors 'none'; base-uri 'none'",
  'referrer-policy': 'no-referrer',
  'x-content-type-options': 'nosniff',
  'x-frame-options': 'DENY',
}

function respond(body: string | null, status: number, extra: Record<string, string> = {}, cookies: string[] = []): Response {
  const headers = new Headers({ ...SECURITY_HEADERS, ...extra })
  if (body !== null) headers.set('content-type', 'text/html; charset=utf-8')
  for (const value of cookies) headers.append('set-cookie', value)
  return new Response(body, { status, headers })
}

const redirect = (location: string, cookies: string[] = []) => respond(null, 303, { location }, cookies)

export async function app(request: Request, env: AppEnv): Promise<Response> {
  const url = new URL(request.url)
  const path = url.pathname.replace(/\/+$/, '') || '/app'

  if (!configured(env)) return respond(notConfiguredPage(), 503)

  if (path === '/app/sign-out') {
    if (request.method !== 'POST') return respond(null, 405, { allow: 'POST' })
    return signOut(request, url)
  }
  if (request.method !== 'GET' && request.method !== 'HEAD') return respond(null, 405, { allow: 'GET, HEAD' })
  if (path === '/app/sign-in') return await startSignIn(url, env)
  if (path === '/app/callback') return await finishSignIn(request, url, env)

  const tokens = readSession(request)
  if (!tokens) {
    // A cookie that is there but unreadable is cleared, not kept around.
    return respond(signInPage(), 200, {}, cookie(request, SESSION_COOKIE) === null ? [] : [clearSession()])
  }
  const caller = new Caller(env, tokens, visitorOf(request))

  if (path === '/app') return finish(caller, await teams(caller), url)
  const team = /^\/app\/teams\/([A-Za-z0-9-]{1,64})$/.exec(path)?.[1]
  if (team) return finish(caller, await members(caller, team), url)
  const me = await caller.directory('me')
  return finish(caller, me.kind === 'ok' ? { status: 404, html: notFoundPage(viewerOf(me.body)) } : me, url)
}

type Rendered = { status: number; html: string } | Exclude<Answer, { kind: 'ok' }>

/// The page, or the sign-in page when the session turned out to be over, with
/// the cookie the response must carry.
function finish(caller: Caller, rendered: Rendered, url: URL): Response {
  if (caller.signedOut || ('kind' in rendered && rendered.kind === 'signed-out')) {
    return respond(signInPage(), 200, {}, [clearSession()])
  }
  const cookies = caller.renewed ? [sessionCookie(caller.tokens)] : []
  if ('html' in rendered) return respond(rendered.html, rendered.status, {}, cookies)
  if (rendered.kind === 'limited') return respond(limitedPage(), 429, { 'retry-after': '60' }, cookies)
  if (rendered.kind === 'refused' && rendered.status === 404) return respond(notFoundPage(), 404, {}, cookies)
  return respond(unreachablePage(url.pathname), 502, {}, cookies)
}

function viewerOf(me: any): Viewer {
  return { email: typeof me?.user?.email === 'string' ? me.user.email : null }
}

async function teams(caller: Caller): Promise<Rendered> {
  const me = await caller.directory('me')
  if (me.kind !== 'ok') return me
  const list: Team[] = Array.isArray(me.body?.teams) ? me.body.teams : []
  return { status: 200, html: teamsPage(list, viewerOf(me.body)) }
}

async function members(caller: Caller, team: string): Promise<Rendered> {
  const me = await caller.directory('me')
  if (me.kind !== 'ok') return me
  const viewer = viewerOf(me.body)
  const found = await caller.directory('teams/members', { team })
  if (found.kind === 'refused' && (found.status === 404 || found.status === 400)) return { status: 404, html: notFoundPage(viewer) }
  if (found.kind === 'refused' && found.status === 403) return { status: 403, html: forbiddenPage(viewer) }
  if (found.kind !== 'ok') return found
  const list: Member[] = Array.isArray(found.body?.members) ? found.body.members : []
  const self = typeof me.body?.user?.id === 'string' ? me.body.user.id : ''
  return { status: 200, html: membersPage(found.body.team ?? { name: 'Team', personal: false }, list, self, viewer) }
}

/// Off to AuthKit, with PKCE and a state bound to this browser.
async function startSignIn(url: URL, env: Configured): Promise<Response> {
  const signIn = await newSignIn()
  const authorize = new URL('https://api.workos.com/user_management/authorize')
  authorize.search = new URLSearchParams({
    client_id: env.APP_WORKOS_CLIENT_ID.trim(),
    redirect_uri: `${url.origin}/app/callback`,
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
  const failed = () => respond(signInFailedPage(), 400, {}, [clearSignIn()])
  if (!pending || !code || !state || state !== pending.state) return failed()

  // Before the relay is asked anything: see `withinLimit`.
  if (!(await withinLimit(env, visitorOf(request)))) return respond(limitedPage(), 429, { 'retry-after': '60' }, [clearSignIn()])
  let tokens
  try {
    tokens = await exchange(env, code, pending.verifier)
  } catch {
    return respond(unreachablePage('/app/sign-in'), 502, {}, [clearSignIn()])
  }
  if (tokens === 'limited') return respond(limitedPage(), 429, { 'retry-after': '60' }, [clearSignIn()])
  if (!tokens) return failed()
  return redirect('/app', [clearSignIn(), sessionCookie(tokens)])
}

/// Forget the session here, then end it at WorkOS too, so the next Sign In
/// asks again rather than walking straight back in on AuthKit's cookie.
///
/// A POST from this site only: a link elsewhere can't sign you out. It calls
/// no /v1/auth route, so it needs no rate limit: the browser goes to WorkOS.
function signOut(request: Request, url: URL): Response {
  const origin = request.headers.get('origin')
  if (origin !== null && origin !== url.origin) return respond(null, 403)
  const session = readSession(request)
  const sid = session ? sessionId(session.access) : null
  if (!sid) return redirect('/app', [clearSession()])
  const logout = new URL('https://api.workos.com/user_management/sessions/logout')
  logout.search = new URLSearchParams({ session_id: sid, return_to: `${url.origin}/app` }).toString()
  return redirect(logout.toString(), [clearSession()])
}
