/// The web app at www.farcooler.com/app (ov-517, team directory phase 3).
///
/// Server-rendered: the Worker signs the visitor in with WorkOS AuthKit, keeps
/// WorkOS's tokens in an HttpOnly cookie (`session.ts`), reads the relay's
/// directory API with them (`relay.ts`), and returns plain HTML (`pages.ts`).
/// The browser never runs our script and never holds a token it can read.
///
/// Signed out, every page is the sign-in page: no route here reads the
/// directory without a session, and the relay checks that session again.

import { handlePost, postOnlyTarget, sessionCookies } from './forms'
import { joinAlreadyPage, joinConfirmPage, joinFailurePage, joinSignInPage } from './join'
import {
  canManage,
  inviteCreatedPage,
  inviteShownPage,
  memberPage,
  readmitConfirmPage,
  notAdminPage,
  removeConfirmPage,
  roleConfirmPage,
  teamPage,
  type Invite,
  type Removed,
  type Runner,
  type TeamView,
} from './manage'
import {
  forbiddenPage,
  limitedPage,
  notConfiguredPage,
  notFoundPage,
  signInFailedPage,
  signInPage,
  signOutEverywhereConfirmPage,
  teamsPage,
  unreachablePage,
  type Member,
  type Team,
  type Viewer,
} from './pages'
import { Caller, configured, exchange, visitorOf, withinLimit, type Answer, type AppEnv, type Configured } from './relay'
import { redirect, respond } from './respond'
import { auditFor, devicesFor, runnerFor, runnersFor } from './views'
import {
  clearSession,
  clearSignIn,
  cookie,
  clearInvite,
  joinCookie,
  readInvite,
  newCsrf,
  newSignIn,
  readJoin,
  readSession,
  readSignIn,
  SESSION_COOKIE,
  sessionCookie,
  signInCookie,
} from './session'

export type { AppEnv } from './relay'

/// Whether a path belongs to the web app.
export function isApp(pathname: string): boolean {
  return pathname === '/app' || pathname.startsWith('/app/')
}

const TEAM = '([A-Za-z0-9-]{1,64})'
const USER = '([A-Za-z0-9_-]{1,128})'
const GET_ROUTES = {
  team: new RegExp(`^/app/teams/${TEAM}$`),
  member: new RegExp(`^/app/teams/${TEAM}/members/${USER}$`),
  role: new RegExp(`^/app/teams/${TEAM}/members/${USER}/role/(admin|member|billing)$`),
  remove: new RegExp(`^/app/teams/${TEAM}/members/${USER}/remove$`),
  join: /^\/app\/join\/([0-9a-f]{64})$/,
  invite: new RegExp(`^/app/teams/${TEAM}/invite$`),
  runners: new RegExp(`^/app/teams/${TEAM}/runners$`),
  runner: new RegExp(`^/app/teams/${TEAM}/runners/([A-Za-z0-9-]{1,64})$`),
  audit: new RegExp(`^/app/teams/${TEAM}/audit$`),
  readmit: new RegExp(`^/app/teams/${TEAM}/members/${USER}/readmit$`),
}

export async function app(request: Request, env: AppEnv): Promise<Response> {
  const url = new URL(request.url)
  const path = url.pathname.replace(/\/+$/, '') || '/app'

  if (!configured(env)) return respond(notConfiguredPage(), 503)

  if (request.method === 'POST') return await handlePost(request, url, path, env)
  if (path === '/app/sign-out') return respond(null, 405, { allow: 'POST' })
  if (request.method !== 'GET' && request.method !== 'HEAD') return respond(null, 405, { allow: 'GET, HEAD' })
  // A path that only takes a form post is not a page: send the person to the
  // page that holds the form, which shows sign-in when they're signed out.
  const holder = postOnlyTarget(path)
  if (holder) return redirect(holder)
  if (path === '/app/sign-in') return await startSignIn(request, url, env)
  if (path === '/app/callback') return await finishSignIn(request, url, env)
  const invite = GET_ROUTES.join.exec(path)?.[1]
  // The code goes into the HttpOnly cookie and out of the address bar at once.
  if (invite) return redirect('/app/join', [joinCookie(invite, readSignIn(request))])

  const tokens = readSession(request)
  if (!tokens) {
    // A cookie that is there but unreadable is cleared, not kept around.
    const stale = cookie(request, SESSION_COOKIE) === null ? [] : [clearSession()]
    return respond(path === '/app/join' && readJoin(request) ? joinSignInPage() : signInPage(), 200, {}, stale)
  }
  // A session from before forms existed has no token yet: it gets one now.
  const minted = !tokens.csrf
  if (minted) tokens.csrf = newCsrf()
  const caller = new Caller(env, tokens, visitorOf(request))
  if (minted) caller.renewed = true
  const csrf = tokens.csrf!

  if (path === '/app') return finish(caller, await teams(caller, csrf, url), url)
  if (path === '/app/join') return await joinPage(request, caller, csrf, url)
  if (path === '/app/sign-out-everywhere') return finish(caller, await confirmSignOutEverywhere(caller, csrf), url)
  let found: RegExpExecArray | null
  if ((found = GET_ROUTES.team.exec(path))) return finish(caller, await teamPageFor(caller, csrf, found[1], url), url)
  if ((found = GET_ROUTES.member.exec(path))) return finish(caller, await memberFor(caller, csrf, found[1], found[2], url, 'member'), url)
  if ((found = GET_ROUTES.role.exec(path))) return finish(caller, await memberFor(caller, csrf, found[1], found[2], url, 'role', found[3]), url)
  if ((found = GET_ROUTES.readmit.exec(path))) return finish(caller, await memberFor(caller, csrf, found[1], found[2], url, 'readmit'), url)
  if ((found = GET_ROUTES.invite.exec(path))) return finish(caller, await inviteFor(request, caller, csrf, found[1]), url)
  if ((found = GET_ROUTES.remove.exec(path))) return finish(caller, await memberFor(caller, csrf, found[1], found[2], url, 'remove'), url)
  const read = /^\/app\/(devices|teams\/)/.test(path) ? await readOnly(caller, csrf, path, url) : null
  if (read) return finish(caller, read, url)
  const me = await caller.directory('me')
  return finish(caller, me.kind === 'ok' ? { status: 404, html: notFoundPage(viewerOf(me.body, csrf)) } : me, url)
}

type Rendered = { status: number; html: string; cookies?: string[] } | { location: string } | Exclude<Answer, { kind: 'ok' }>

/// The page, or the sign-in page when the session turned out to be over, with
/// the cookie the response must carry.
function finish(caller: Caller, rendered: Rendered, url: URL): Response {
  if (caller.signedOut || ('kind' in rendered && rendered.kind === 'signed-out')) {
    return respond(signInPage(), 200, {}, [clearSession()])
  }
  const cookies = sessionCookies(caller)
  if ('location' in rendered) return redirect(rendered.location, cookies)
  if ('html' in rendered) return respond(rendered.html, rendered.status, {}, [...cookies, ...(rendered.cookies ?? [])])
  if (rendered.kind === 'limited') return respond(limitedPage(), 429, { 'retry-after': '60' }, cookies)
  if (rendered.kind === 'refused' && rendered.status === 404) return respond(notFoundPage(), 404, {}, cookies)
  return respond(unreachablePage(url.pathname), 502, {}, cookies)
}

/// The read-only pages (`views.ts`), or null for a path that isn't one.
async function readOnly(caller: Caller, csrf: string, path: string, url: URL): Promise<Rendered | null> {
  const found = [GET_ROUTES.runners, GET_ROUTES.runner, GET_ROUTES.audit].map(r => r.exec(path)).find(Boolean) ?? (path === '/app/devices' ? [] : null)
  if (!found) return null
  const me = await caller.directory('me')
  if (me.kind !== 'ok') return me
  const viewer = viewerOf(me.body, csrf)
  if (path === '/app/devices') return devicesFor(viewer, me.body)
  if (GET_ROUTES.runner.test(path)) return await runnerFor(caller, viewer, me.body, found[1], found[2])
  if (GET_ROUTES.runners.test(path)) return await runnersFor(caller, viewer, me.body, found[1])
  return await auditFor(caller, viewer, me.body, found[1], url.searchParams.get('before'))
}

function viewerOf(me: any, csrf: string): Viewer {
  return { email: typeof me?.user?.email === 'string' ? me.user.email : null, csrf }
}

/// The page that asks first. A GET changes nothing; the form on it POSTs.
async function confirmSignOutEverywhere(caller: Caller, csrf: string): Promise<Rendered> {
  const me = await caller.directory('me')
  if (me.kind !== 'ok') return me
  return { status: 200, html: signOutEverywhereConfirmPage(viewerOf(me.body, csrf)) }
}

const noticeOf = (url: URL) => url.searchParams.get('notice')

async function teams(caller: Caller, csrf: string, url: URL): Promise<Rendered> {
  const me = await caller.directory('me')
  if (me.kind !== 'ok') return me
  const list: Team[] = Array.isArray(me.body?.teams) ? me.body.teams : []
  return { status: 200, html: teamsPage(list, viewerOf(me.body, csrf), noticeOf(url)) }
}

/// What a team page needs, or the page to show instead. An admin of a team
/// with other people in it also gets its invites.
async function loadTeam(caller: Caller, csrf: string, team: string, url: URL): Promise<TeamView | Rendered> {
  const me = await caller.directory('me')
  if (me.kind !== 'ok') return me
  const viewer = viewerOf(me.body, csrf)
  const found = await caller.directory('teams/members', { team })
  if (found.kind === 'refused' && (found.status === 404 || found.status === 400)) return { status: 404, html: notFoundPage(viewer) }
  if (found.kind === 'refused' && found.status === 403) return { status: 403, html: forbiddenPage(viewer) }
  if (found.kind !== 'ok') return found
  const members: Member[] = Array.isArray(found.body?.members) ? found.body.members : []
  const mine = (Array.isArray(me.body?.teams) ? me.body.teams : []).find((t: Team) => t.id === team)
  const view: TeamView = {
    team: { id: team, name: found.body.team?.name ?? 'Team', personal: found.body.team?.personal === true },
    members,
    removed: Array.isArray(found.body?.removed) ? (found.body.removed as Removed[]) : [],
    invites: [],
    self: typeof me.body?.user?.id === 'string' ? me.body.user.id : '',
    role: typeof mine?.role === 'string' ? mine.role : 'member',
    viewer,
    notice: noticeOf(url),
    now: Date.now(),
  }
  if (canManage(view)) {
    const invites = await caller.directory('teams/invites/list', { team })
    // A list that didn't load is not an empty one.
    if (invites.kind !== 'ok') return invites
    view.invites = Array.isArray(invites.body?.invites) ? (invites.body.invites as Invite[]) : []
  }
  return view
}

const isView = (loaded: TeamView | Rendered): loaded is TeamView => 'team' in loaded && 'members' in loaded

async function teamPageFor(caller: Caller, csrf: string, team: string, url: URL): Promise<Rendered> {
  const loaded = await loadTeam(caller, csrf, team, url)
  return isView(loaded) ? { status: 200, html: teamPage(loaded) } : loaded
}

/// A member's page, or the confirm step for one change to them.
async function memberFor(
  caller: Caller,
  csrf: string,
  team: string,
  user: string,
  url: URL,
  kind: 'member' | 'role' | 'remove' | 'readmit',
  role = '',
): Promise<Rendered> {
  const loaded = await loadTeam(caller, csrf, team, url)
  if (!isView(loaded)) return loaded
  const here = `/app/teams/${encodeURIComponent(team)}`
  // A personal team has no one to manage; anyone but an admin has no controls.
  if (loaded.team.personal) return { location: here }
  if (!canManage(loaded)) return { status: 403, html: notAdminPage(loaded.viewer) }
  if (kind === 'readmit') {
    const who = loaded.removed.find(r => r.user === user)
    return who ? { status: 200, html: readmitConfirmPage(loaded, who) } : { location: `${here}?notice=err-gone` }
  }
  const member = loaded.members.find(m => m.user === user)
  if (!member) return { location: `${here}?notice=err-gone` }
  const page = `${here}/members/${encodeURIComponent(user)}`
  if (kind === 'member') return { status: 200, html: memberPage(loaded, member) }
  const only = member.role === 'admin' && loaded.members.filter(m => m.role === 'admin').length <= 1
  if (only || (kind === 'remove' && member.user === loaded.self)) return { location: page }
  if (kind === 'role') {
    return member.role === role ? { location: page } : { status: 200, html: roleConfirmPage(loaded, member, role) }
  }
  const runners = await caller.directory('teams/runners', { team })
  // A runner list that didn't load must not read as "holds no role anywhere".
  if (runners.kind !== 'ok') return runners
  const list: Runner[] = Array.isArray(runners.body?.runners) ? runners.body.runners : []
  return { status: 200, html: removeConfirmPage(loaded, member, list) }
}

/// The new invite link, once. The code came in a cookie from the POST that made
/// it, and goes out with this response: a reload finds nothing to show.
async function inviteFor(request: Request, caller: Caller, csrf: string, team: string): Promise<Rendered> {
  const me = await caller.directory('me')
  if (me.kind !== 'ok') return me
  const viewer = viewerOf(me.body, csrf)
  const made = readInvite(request)
  if (!made || made.team !== team) return { status: 200, html: inviteShownPage(team, viewer), cookies: made ? [] : [clearInvite()] }
  const link = `${new URL(request.url).origin}/app/join/${made.code}`
  return { status: 200, html: inviteCreatedPage(made, link, viewer), cookies: [clearInvite()] }
}

/// The join confirmation: who is asking, and what the code in the cookie is for.
async function joinPage(request: Request, caller: Caller, csrf: string, url: URL): Promise<Response> {
  const code = readJoin(request)
  const me = await caller.directory('me')
  if (me.kind !== 'ok') return finish(caller, me, url)
  const viewer = viewerOf(me.body, csrf)
  if (!code) return finish(caller, { status: 404, html: joinFailurePage('unknown', viewer) }, url)
  const preview = await caller.directory('invites/preview', { code })
  if (preview.kind === 'ok' && typeof preview.body?.team?.name === 'string') {
    const team = preview.body.team
    return finish(caller, { status: 200, html: preview.body.member === true ? joinAlreadyPage(team, viewer) : joinConfirmPage(team.name, String(preview.body.role), viewer, typeof preview.body.invitedBy === 'string' ? preview.body.invitedBy : null) }, url)
  }
  if (preview.kind === 'refused') {
    const reason = preview.status === 403 ? 'domain' : (preview.reason ?? 'unknown')
    return finish(caller, { status: preview.status, html: joinFailurePage(reason, viewer) }, url)
  }
  return finish(caller, preview.kind === 'ok' ? { kind: 'unreachable' } : preview, url)
}

/// Off to AuthKit, with PKCE and a state bound to this browser.
async function startSignIn(request: Request, url: URL, env: Configured): Promise<Response> {
  // A join link opened first has left its code in the cookie; it goes on with
  // the sign-in, still out of every URL.
  const signIn = { ...(await newSignIn()), join: readJoin(request) ?? undefined }
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
  const session = sessionCookie({ ...tokens, csrf: newCsrf() })
  // On to the join confirmation when a link brought them here. Its code
  // stays in the cookie for the Join button, and nowhere else.
  if (pending.join) return redirect('/app/join', [joinCookie(pending.join), session])
  return redirect('/app', [clearSignIn(), session])
}
