/// `/api/relay/<route>`: the page's only way to the relay.
///
/// The browser never holds a WorkOS token. It posts here, and the Worker calls
/// the relay with the bearer from the HttpOnly session cookie, through the same
/// `Caller` the site's /app uses (services/common/relay.ts). The checks, in
/// order, each with a test that goes red without it (test/api.test.ts):
///
/// 1. CSRF: the request carries `x-fc: 1` and an `Origin` that is exactly this
///    host. A cross-site form can set neither, and a cross-site fetch that set
///    the header would need a CORS preflight this Worker never answers.
/// 2. The route is on `ROUTES`. Nothing else is forwarded, whatever the relay
///    would accept from this session.
/// 3. A session, and an account the flag admits (flag.ts).

import { Caller, visitorOf, type Answer, type Configured } from '../../common/relay'
import { clearSession, readSession, sessionCookie } from '../../common/session'
import type { WebEnv } from './env'
import { admitted } from './flag'
import { withHeaders } from './headers'

/// The relay routes the page may call, under /v1/. Slice 1 needs only who is
/// signed in, for the enrollment offer's account.
///
/// With WEB_CLIENT "everyone" a route here is reachable by every signed-in
/// stranger, so any new route needs its own security review (ov-810).
export const ROUTES = new Set(['directory/me'])

const MAX_BODY = 16 * 1024

function json(body: unknown, status: number, cookies: string[] = []): Response {
  return withHeaders(JSON.stringify(body), status, { 'cache-control': 'no-store', 'content-type': 'application/json' }, cookies)
}

export async function api(request: Request, url: URL, env: Configured & WebEnv): Promise<Response> {
  if (request.method !== 'POST') return withHeaders(null, 405, { allow: 'POST' })
  if (request.headers.get('x-fc') !== '1' || request.headers.get('origin') !== url.origin) return json({ error: 'forbidden' }, 403)
  const route = url.pathname.slice('/api/relay/'.length)
  if (!ROUTES.has(route)) return json({ error: 'not found' }, 404)

  const tokens = readSession(request)
  if (!tokens) return json({ error: 'signed out' }, 401)
  const caller = new Caller(env, tokens, visitorOf(request))
  const cookies = () => (caller.renewed && !caller.signedOut ? [sessionCookie(caller.tokens)] : [])

  const me = await caller.directory('me')
  if (me.kind !== 'ok') return failure(me, caller)
  if (!admitted(env, me.body?.user?.id)) return json({ error: 'not found' }, 404, cookies())

  let answer: Answer = me
  if (route !== 'directory/me') {
    const text = await request.text()
    if (text.length > MAX_BODY) return json({ error: 'too large' }, 413, cookies())
    let body: unknown
    try {
      body = text ? JSON.parse(text) : {}
    } catch {
      return json({ error: 'bad request' }, 400, cookies())
    }
    if (!body || typeof body !== 'object' || Array.isArray(body)) return json({ error: 'bad request' }, 400, cookies())
    const [kind, ...rest] = route.split('/')
    const sub = rest.join('/')
    answer = kind === 'account' ? await caller.account(sub, body as Record<string, unknown>) : await caller.directory(sub, body as Record<string, unknown>)
  }
  if (answer.kind === 'ok') return json(answer.body, 200, cookies())
  return failure(answer, caller)
}

function failure(answer: Exclude<Answer, { kind: 'ok' }>, caller: Caller): Response {
  const renewed = caller.renewed && !caller.signedOut ? [sessionCookie(caller.tokens)] : []
  switch (answer.kind) {
    case 'signed-out':
      return json({ error: 'signed out' }, 401, [clearSession()])
    case 'limited':
      return withHeaders(JSON.stringify({ error: 'limited' }), 429, { 'cache-control': 'no-store', 'content-type': 'application/json', 'retry-after': '60' }, renewed)
    case 'refused':
      return json({ error: answer.error ?? 'refused', ...(answer.reason ? { reason: answer.reason } : {}) }, answer.status, renewed)
    default:
      return json({ error: 'unreachable' }, 502, renewed)
  }
}
