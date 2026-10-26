/// Who is calling: a verified WorkOS session, checked against the account.

import { json, type Env } from './index'
import { verifySession, type Session } from './workos'

/// The signed-in account, or the response to send instead. `limit`, when
/// given, is `requireSession`'s.
export async function requireAccount(request: Request, env: Env, limit?: RateLimit): Promise<string | Response> {
  const session = await requireSession(request, env, limit)
  return session instanceof Response ? session : session.userId
}

/// How long after the stamp a token's `iat` still counts as before it. WorkOS
/// stamps `iat` with its own clock, in whole seconds; this is the skew and the
/// rounding the refusal allows for. It errs toward refusing, which costs a
/// person who signs in within two seconds of signing out one more sign-in.
export const IAT_MARGIN_MS = 2000

/// The verified session, or the response to send instead.
///
/// Refuses a token from a session that existed when the account last signed
/// out everywhere (ov-564), by two tests, either of which is enough:
///
/// - **Its `sid` was first seen at or before the stamp.** A refresh keeps the
///   `sid` and mints a new `iat`, so this is what stops a stolen refresh token
///   whatever the clock says. Sign-out records every session WorkOS lists, so
///   one this relay never saw before is covered too.
/// - **Its `iat` is within `IAT_MARGIN_MS` of the stamp or before it.** An
///   access token is checked against WorkOS's keys without a call to WorkOS,
///   so ending the sessions upstream alone leaves every copy working until it
///   expires. This covers a token with no `sid`.
///
/// This and `workos.ts` are the only files that call `verifySession`: a route
/// that verified a token itself would skip both tests.
///
/// `limit`, when a route passes one, is a per-account rate limit, keyed on the
/// verified `sub` and spent before any D1 read: a refused call costs the
/// signature check and nothing else (ov-815). Keyed on the account, so one
/// account's loop spends its own budget and no one else's. Open when the
/// binding is absent, as `AUTH_LIMIT` is (`withinRate` says why).
export async function requireSession(request: Request, env: Env, limit?: RateLimit): Promise<Session | Response> {
  const header = request.headers.get('authorization') ?? ''
  if (!header.startsWith('Bearer ')) return json({ error: 'unauthorized' }, 401)

  const session = await verifySession(header.slice(7), env)
  if (!session) return json({ error: 'unauthorized' }, 401)
  if (limit && !(await limit.limit({ key: session.userId })).success) return json({ error: 'slow down' }, 429)

  // First sight of an account creates it. There is no signup step to get wrong
  // and no window where someone is authenticated but has nowhere to be stored.
  // COALESCE, because a WorkOS access token carries no `email` claim: without
  // it every request erased the email sign-in stored.
  //
  // Read first, and write only when there's something to write: the account
  // is new, or the token carries an email the row doesn't have. The upsert
  // alone wrote a row on every signed-in request, which any account could loop
  // on (ov-811). The upsert still settles a race between two first requests.
  const known = await env.DB.prepare(`SELECT sessions_revoked_at, email FROM accounts WHERE id = ?`)
    .bind(session.userId)
    .first<{ sessions_revoked_at: number | null; email: string | null }>()
  const unchanged = known !== null && (session.email == null || session.email === known.email)
  const account = unchanged
    ? known
    : await env.DB.prepare(
        `INSERT INTO accounts (id, created_at, email) VALUES (?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET email = COALESCE(excluded.email, accounts.email)
         RETURNING sessions_revoked_at`,
      )
        .bind(session.userId, Date.now(), session.email ?? null)
        .first<{ sessions_revoked_at: number | null }>()
  const revokedAt = account?.sessions_revoked_at ?? null

  // WorkOS always sets `sid`. A token without one can only be cut off by its
  // `iat`, which a refresh renews, so it is no session at all.
  if (!session.sessionId) return json({ error: 'unauthorized' }, 401)
  const firstSeen = await sessionFirstSeen(env, session.userId, session.sessionId)
  if (revokedAt !== null && firstSeen <= revokedAt) return json({ error: 'unauthorized' }, 401)
  if (revokedAt !== null && session.issuedAt * 1000 <= revokedAt + IAT_MARGIN_MS) return json({ error: 'unauthorized' }, 401)

  return session
}

/// When this relay first saw `sid`, recording it now if never.
async function sessionFirstSeen(env: Env, account: string, sid: string): Promise<number> {
  const known = await env.DB.prepare(`SELECT first_seen FROM sessions_seen WHERE account_id = ? AND sid = ?`)
    .bind(account, sid)
    .first<{ first_seen: number }>()
  if (known) return known.first_seen
  const now = Date.now()
  await env.DB.prepare(`INSERT OR IGNORE INTO sessions_seen (sid, account_id, first_seen) VALUES (?, ?, ?)`)
    .bind(sid, account, now)
    .run()
  // Read back, so two first requests that raced agree on the earlier time.
  const row = await env.DB.prepare(`SELECT first_seen FROM sessions_seen WHERE account_id = ? AND sid = ?`)
    .bind(account, sid)
    .first<{ first_seen: number }>()
  return row?.first_seen ?? now
}
