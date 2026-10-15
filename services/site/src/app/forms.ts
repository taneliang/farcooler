/// Everything that changes something: the web app's POSTs (ov-524).
///
/// Three checks stand before any of them, in this order, and each is a guard
/// with a test that goes red without it:
///
/// 1. Same origin. The request names this site in `Origin`, or has no `Origin`
///    and a `Sec-Fetch-Site: same-origin`. A form on another site is refused.
/// 2. A session. Signed out, there is nothing to change.
/// 3. The session's CSRF token (`session.ts`), sent in the form body. The
///    cookie travels with a cross-site form post by default (SameSite=Lax
///    doesn't stop a top-level POST in every browser path); the token does not.
///
/// Whether the person may do the thing is the relay's answer, not this file's:
/// a forged POST from a member gets the relay's 403.

import { notAdminPage, refusedPage } from './manage'
import { joinAlreadyPage, joinFailurePage } from './join'
import { limitedPage, notFoundPage, signedOutEverywherePage, signInPage, unreachablePage, type Viewer } from './pages'
import { Caller, visitorOf, type Answer, type Configured } from './relay'
import { redirect, respond } from './respond'
import { clearInvite, clearSession, clearSignIn, cookie, csrfMatches, inviteCookie, readJoin, readSession, SESSION_COOKIE, sessionCookie, sessionId } from './session'

const TEAM = '([A-Za-z0-9-]{1,64})'
const USER = '([A-Za-z0-9_-]{1,128})'
const ROUTES = {
  signOut: /^\/app\/sign-out$/,
  signOutEverywhere: /^\/app\/sign-out-everywhere$/,
  join: /^\/app\/join$/,
  createTeam: /^\/app\/teams$/,
  createInvite: new RegExp(`^/app/teams/${TEAM}/invites$`),
  revokeInvite: new RegExp(`^/app/teams/${TEAM}/invites/${TEAM}/revoke$`),
  setRole: new RegExp(`^/app/teams/${TEAM}/members/${USER}/role$`),
  remove: new RegExp(`^/app/teams/${TEAM}/members/${USER}/remove$`),
  readmit: new RegExp(`^/app/teams/${TEAM}/members/${USER}/readmit$`),
}

/// Whether the request comes from this site's own pages.
export function sameOrigin(request: Request, url: URL): boolean {
  const site = request.headers.get('sec-fetch-site')
  // A browser that says the request is cross-site is believed over a header
  // that says otherwise.
  if (site !== null && site !== 'same-origin') return false
  const origin = request.headers.get('origin')
  if (origin !== null) return origin === url.origin
  return site === 'same-origin'
}

/// The cookies a response must carry for the session as the caller left it.
export function sessionCookies(caller: Caller): string[] {
  return caller.signedOut ? [clearSession()] : caller.renewed ? [sessionCookie(caller.tokens)] : []
}

async function formOf(request: Request): Promise<URLSearchParams> {
  if (!(request.headers.get('content-type') ?? '').toLowerCase().startsWith('application/x-www-form-urlencoded')) return new URLSearchParams()
  const body = await request.text()
  return new URLSearchParams(body.length > 4096 ? '' : body)
}

/// Whether this POST is one the app knows.
export function isPost(path: string): boolean {
  return Object.values(ROUTES).some(route => route.test(path))
}

export async function handlePost(request: Request, url: URL, path: string, env: Configured): Promise<Response> {
  if (!isPost(path)) return respond(null, 405, { allow: 'GET, HEAD' })
  if (!sameOrigin(request, url)) return respond(refusedPage(), 403)

  const tokens = readSession(request)
  if (!tokens) {
    // Signing out with nothing to sign out of is done. Anything else needs a session.
    const stale = cookie(request, SESSION_COOKIE) === null ? [] : [clearSession()]
    return ROUTES.signOut.test(path) ? redirect('/app', [...stale, clearSignIn(), clearInvite()]) : respond(signInPage(), 401, {}, stale)
  }
  const form = await formOf(request)
  if (!csrfMatches(tokens, form.get('csrf'))) return respond(refusedPage(), 403)

  if (ROUTES.signOut.test(path)) return signOut(tokens.access, url)
  const caller = new Caller(env, tokens, visitorOf(request))
  const csrf = tokens.csrf ?? ''
  const done = (response: (cookies: string[]) => Response) => response(sessionCookies(caller))

  if (ROUTES.signOutEverywhere.test(path)) return await signOutEverywhere(caller, csrf)

  let match: RegExpExecArray | null
  if (ROUTES.createTeam.test(path)) {
    const made = await caller.directory('teams/create', { name: form.get('name') ?? '' })
    if (made.kind === 'ok' && typeof made.body?.team?.id === 'string') {
      return done(cookies => redirect(`/app/teams/${encodeURIComponent(made.body.team.id)}?notice=team-created`, cookies))
    }
    return await failed(caller, made, '/app', csrf, '/app')
  }
  if (ROUTES.join.test(path)) return await join(request, caller, csrf)

  if ((match = ROUTES.createInvite.exec(path))) {
    const team = match[1]
    const domain = (form.get('domain') ?? '').trim()
    const made = await caller.directory('teams/invites/create', {
      team,
      role: form.get('role') ?? 'member',
      days: Number(form.get('days')),
      uses: Number(form.get('uses')),
      ...(domain ? { domain } : {}),
    })
    if (made.kind !== 'ok' || typeof made.body?.code !== 'string') return await failed(caller, made, `/app/teams/${team}`, csrf, `/app/teams/${team}`)
    // The team's name, for the page. The invite is made either way.
    const named = await caller.directory('teams/members', { team })
    const info = named.kind === 'ok' && typeof named.body?.team?.name === 'string' ? named.body.team.name : 'the team'
    // Post, redirect, get: the code goes in a short-lived HttpOnly cookie to the
    // page that shows it once, so a reload makes no second invite and no URL
    // ever holds it.
    const invited = inviteCookie({
      code: made.body.code,
      team,
      name: info,
      role: form.get('role') === 'billing' ? 'billing' : 'member',
      days: Number(form.get('days')),
      uses: Number(form.get('uses')),
      expiresAt: Number(made.body.expiresAt),
      domain: domain ? domain.toLowerCase() : null,
    })
    return done(cookies => redirect(`/app/teams/${encodeURIComponent(team)}/invite`, [...cookies, invited]))
  }
  if ((match = ROUTES.revokeInvite.exec(path))) {
    return await change(caller, 'teams/invites/revoke', { team: match[1], invite: match[2] }, match[1], 'revoked', csrf)
  }
  if ((match = ROUTES.setRole.exec(path))) {
    return await change(caller, 'teams/members/role', { team: match[1], user: match[2], role: form.get('role') ?? '' }, match[1], 'role-changed', csrf)
  }
  if ((match = ROUTES.remove.exec(path))) {
    return await change(caller, 'teams/members/remove', { team: match[1], user: match[2] }, match[1], 'removed', csrf)
  }
  match = ROUTES.readmit.exec(path)!
  return await change(caller, 'teams/members/readmit', { team: match[1], user: match[2] }, match[1], 'readmitted', csrf)
}

/// One change to a team, then back to the team page saying what happened.
async function change(caller: Caller, route: string, body: Record<string, unknown>, team: string, notice: string, csrf: string): Promise<Response> {
  const answer = await caller.directory(route, body)
  const page = `/app/teams/${team}`
  if (answer.kind === 'ok') return redirect(`${page}?notice=${notice}`, sessionCookies(caller))
  return await failed(caller, answer, page, csrf, page)
}

/// The notice for a refusal the person can act on, by what the relay said.
function noticeFor(answer: Extract<Answer, { kind: 'refused' }>): string {
  if (answer.status === 409) return answer.error === 'personal' ? 'err-personal' : 'err-last-admin'
  if (answer.status === 404) return 'err-gone'
  if (answer.status === 400) {
    if (answer.error === 'user') return 'err-gone'
    if (answer.error && ['name', 'domain', 'days', 'uses', 'role'].includes(answer.error)) return `err-${answer.error}`
  }
  return 'err-failed'
}

/// A directory call that didn't work, as the page that says so.
async function failed(caller: Caller, answer: Answer, page: string, csrf: string, retry: string): Promise<Response> {
  const cookies = sessionCookies(caller)
  if (caller.signedOut || answer.kind === 'signed-out') return respond(signInPage(), 200, {}, [clearSession()])
  if (answer.kind === 'limited') return respond(limitedPage(), 429, { 'retry-after': '60' }, cookies)
  if (answer.kind === 'unreachable' || answer.kind === 'ok') return respond(unreachablePage(retry), 502, {}, cookies)
  if (answer.status === 403) return respond(notAdminPage(await viewerOf(caller, csrf)), 403, {}, cookies)
  if (answer.status === 404 && page === '/app') return respond(notFoundPage(await viewerOf(caller, csrf)), 404, {}, cookies)
  return redirect(`${page}?notice=${noticeFor(answer)}`, cookies)
}

async function viewerOf(caller: Caller, csrf: string): Promise<Viewer> {
  const me = await caller.directory('me')
  return { email: me.kind === 'ok' && typeof me.body?.user?.email === 'string' ? me.body.user.email : null, csrf }
}

/// Accept the invite waiting in the sign-in cookie.
async function join(request: Request, caller: Caller, csrf: string): Promise<Response> {
  const code = readJoin(request)
  const viewer = await viewerOf(caller, csrf)
  if (!code) return respond(joinFailurePage('unknown', viewer), 404, {}, sessionCookies(caller))
  const answer = await caller.directory('invites/accept', { code })
  const cookies = [...sessionCookies(caller), clearSignIn()]
  if (answer.kind === 'ok' && typeof answer.body?.team?.id === 'string') {
    if (answer.body.joined === false) return respond(joinAlreadyPage(answer.body.team, viewer), 200, {}, cookies)
    return redirect(`/app/teams/${encodeURIComponent(answer.body.team.id)}?notice=joined`, cookies)
  }
  if (answer.kind === 'refused') {
    const reason = answer.status === 403 ? 'domain' : (answer.reason ?? 'unknown')
    return respond(joinFailurePage(reason, viewer), answer.status, {}, cookies)
  }
  // Not a verdict on the link: keep the code so Try Again can use it.
  return await failed(caller, answer, '/app/join', csrf, '/app/join')
}

/// The labels of the runners the relay says were paired lately, whatever shape
/// the reply has.
function runnerLabels(runners: unknown): string[] {
  if (!Array.isArray(runners)) return []
  return runners.map(r => r?.label).filter((l): l is string => typeof l === 'string' && l.length > 0)
}

/// End every session on the account (ov-564). The relay refuses this browser's
/// token from then on, so the cookies go too, along with any join code or new
/// invite waiting here, and the page says so rather than showing a sign-in
/// page that reads as a mistake. Where the relay couldn't confirm every session
/// upstream, the page says that and what to do.
async function signOutEverywhere(caller: Caller, csrf: string): Promise<Response> {
  const answer = await caller.account('signout-all')
  const retry = '/app/sign-out-everywhere'
  if (answer.kind === 'ok') {
    return respond(signedOutEverywherePage(answer.body?.complete === true, runnerLabels(answer.body?.runners)), 200, {}, [clearSession(), clearSignIn(), clearInvite()])
  }
  if (answer.kind === 'refused') return respond(unreachablePage(retry, await viewerOf(caller, csrf)), 502, {}, sessionCookies(caller))
  return await failed(caller, answer, retry, csrf, retry)
}

/// Forget the session here, and any join code or new invite waiting in this
/// browser (the next person to sign in must not inherit them), then end it at WorkOS too, so the next Sign In
/// asks again rather than walking straight back in on AuthKit's cookie.
function signOut(access: string, url: URL): Response {
  const sid = sessionId(access)
  if (!sid) return redirect('/app', [clearSession(), clearSignIn(), clearInvite()])
  const logout = new URL('https://api.workos.com/user_management/sessions/logout')
  logout.search = new URLSearchParams({ session_id: sid, return_to: `${url.origin}/app` }).toString()
  return redirect(logout.toString(), [clearSession(), clearSignIn(), clearInvite()])
}
