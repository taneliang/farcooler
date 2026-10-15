/// Sign out everywhere (ov-564): end every session an account has.
///
/// Why it exists: a stolen session can register a device and, on an account
/// with no confirmed device, win first-use trust (R-75). Removing the
/// impostor's device doesn't end its session, so it can register again. This
/// is the one thing that does.
///
/// Two halves, because a session is two things:
///
/// - **Access tokens** are signed JWTs this relay checks without calling
///   WorkOS, so revoking upstream leaves every copy working until it expires.
///   Stamping `accounts.sessions_revoked_at` is what stops them, at once, on
///   every route that needs a session (`requireSession`).
/// - **Refresh tokens** are opaque to the relay and live at WorkOS, so only
///   WorkOS can end them: this lists the user's sessions there and revokes
///   each. The stamp is written first and the audit line with it, so a WorkOS
///   outage still ends every access token and leaves a record. The answer
///   then says `complete: false`, and asking again retries the WorkOS half.
///
/// Not ended: a runner's daemon token. That is a pairing the person made, not
/// a session, and `daemons/revoke` removes it.

import { audit } from './directory/common'
import { forgetInstall, shownLabel, tellAccount } from './directory/notices'
import { json, type Env } from './index'
import { requireSession } from './session'

/// WorkOS lists 100 at most per page. Five pages is more sessions than a
/// person has; past it the answer says `complete: false` rather than looping.
const PAGE = 100
const MAX_PAGES = 5

export async function signOutEverywhere(request: Request, env: Env): Promise<Response> {
  const session = await requireSession(request, env)
  if (session instanceof Response) return session
  const account = session.userId

  // This install's push row goes first, so the notice below reaches the other
  // apps and not the one that is signing out. Possession of the token is the
  // authority, as at `logout`.
  let body: { pushToken?: unknown } = {}
  try {
    const parsed = await request.json()
    if (parsed && typeof parsed === 'object') body = parsed as { pushToken?: unknown }
  } catch {
    // No body is fine: it only carries the optional push token.
  }
  await forgetInstall(env, body.pushToken)

  // The window the runner list covers: since the last time the person did
  // this, or the last 30 days when they never have.
  const before = await env.DB.prepare(`SELECT sessions_revoked_at FROM accounts WHERE id = ?`)
    .bind(account)
    .first<{ sessions_revoked_at: number | null }>()

  const stamped = Date.now()
  await env.DB.batch([
    stampStatement(env, account, stamped),
    audit(env, { team: null, actor: account, action: 'sessions.revoked_all' }),
  ])

  const ended = await endAtWorkOS(env, account, session.sessionId)

  // Every session WorkOS listed counts as seen at the stamp, so a refresh of
  // one this relay never heard from still carries a `sid` first seen at or
  // before it. Then the stamp moves to now: a session first seen while the
  // WorkOS calls ran is refused too.
  const seen = [...ended.ids].map(sid =>
    env.DB.prepare(
      `INSERT INTO sessions_seen (sid, account_id, first_seen) VALUES (?, ?, ?)
       ON CONFLICT (account_id, sid) DO UPDATE SET first_seen = MIN(first_seen, excluded.first_seen)`,
    ).bind(sid, account, stamped),
  )
  await env.DB.batch([...seen, stampStatement(env, account, Date.now())])

  const runners = await pairedSince(env, account, before?.sessions_revoked_at ?? stamped - RECENT_MS)

  // The notice goes first, and then every push, pulse and Live Activity
  // credential for the account goes: a pulse token or push row from before the
  // stamp would otherwise go on reading the account's runners and fleet. The
  // apps register again when the person signs in.
  await tellAccount(env, account, notice(runners))
  await env.DB.batch([
    env.DB.prepare(`DELETE FROM devices WHERE account_id = ?`).bind(account),
    env.DB.prepare(`DELETE FROM live_activities WHERE account_id = ?`).bind(account),
    env.DB.prepare(`DELETE FROM install_cards WHERE account_id = ?`).bind(account),
  ])
  return json({ ok: true, complete: ended.complete, ended: ended.count, runners })
}

/// Raise the stamp to `at`, never lower it.
function stampStatement(env: Env, account: string, at: number): D1PreparedStatement {
  return env.DB.prepare(`UPDATE accounts SET sessions_revoked_at = MAX(COALESCE(sessions_revoked_at, 0), ?) WHERE id = ?`).bind(at, account)
}

/// A runner pairing made recently enough that a stolen session may have made
/// it. Pairings aren't sessions and stay; the person decides.
export interface PairedRunner {
  id: string
  label: string
  pairedAt: number
}

const RECENT_MS = 30 * 24 * 60 * 60 * 1000

async function pairedSince(env: Env, account: string, since: number): Promise<PairedRunner[]> {
  const rows = await env.DB.prepare(
    `SELECT id, label, created_at FROM daemons WHERE account_id = ? AND created_at > ? ORDER BY created_at DESC LIMIT 20`,
  )
    .bind(account, since)
    .all<{ id: string; label: string; created_at: number }>()
  return (rows.results ?? []).map(r => ({ id: r.id, label: shownLabel(r.label), pairedAt: r.created_at }))
}

/// What the other apps are told. A runner's name is a label whoever paired it
/// chose, so it goes through `shownLabel` like a device's.
function notice(runners: PairedRunner[]): { title: string; subtitle: string } {
  const names = runners.slice(0, 3).map(r => r.label).join(', ')
  return {
    title: 'You were signed out everywhere',
    subtitle: runners.length
      ? `Every session on your Far Cooler account ended. Runners paired recently: ${names}. Sign in and remove any you don’t recognize.`
      : 'Every session on your Far Cooler account ended. If that wasn’t you, sign in again and check your devices.',
  }
}

/// Revoke each of the user's WorkOS sessions, and the caller's own by its
/// `sid` in case the list misses it. Never throws and never quotes a reply:
/// WorkOS's errors can echo what was sent.
async function endAtWorkOS(env: Env, user: string, own?: string): Promise<{ complete: boolean; count: number; ids: Set<string> }> {
  const headers = { 'content-type': 'application/json', authorization: `Bearer ${env.WORKOS_API_KEY}` }
  const ids = new Set<string>(own ? [own] : [])
  let complete = true

  let after: string | null = null
  for (let page = 0; ; page++) {
    if (page === MAX_PAGES) {
      complete = false
      break
    }
    try {
      const query = `limit=${PAGE}&order=desc${after ? `&after=${encodeURIComponent(after)}` : ''}`
      const response = await fetch(
        `https://api.workos.com/user_management/users/${encodeURIComponent(user)}/sessions?${query}`,
        { headers },
      )
      if (!response.ok) {
        complete = false
        break
      }
      const list = await response.json<{ data?: { id?: unknown; status?: unknown }[]; list_metadata?: { after?: unknown } }>()
      for (const item of list.data ?? []) {
        if (typeof item.id === 'string' && item.status !== 'revoked') ids.add(item.id)
      }
      const next = list.list_metadata?.after
      if (typeof next !== 'string' || !next) break
      after = next
    } catch {
      complete = false
      break
    }
  }

  let count = 0
  for (const id of ids) {
    try {
      const response = await fetch('https://api.workos.com/user_management/sessions/revoke', {
        method: 'POST',
        headers,
        body: JSON.stringify({ session_id: id }),
      })
      if (response.ok) count += 1
      else complete = false
    } catch {
      complete = false
    }
  }
  return { complete, count, ids }
}
