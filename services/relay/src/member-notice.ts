/// A runner's push to one member of its team (ov-646, option A of
/// `.claude/agent/reports/ov-646/design.md`, approved by the owner Oct 22).
///
/// `/v1/notify` names no destination, so a runner can only ever reach the
/// account that registered it. When a member's own Claude login stops working
/// on a runner, that member is the one person who can fix it, so this route
/// lets the runner name them, and the relay decides whether they may be named:
///
///   - The token must be a registered runner's (`runners.daemon_id`). A token
///     paired the old way through `/v1/daemons` has no runner and no team, so
///     it can name nobody.
///   - The account must be an `admin` or `member` of that runner's team, the
///     team must not be a personal one, and the account must not be the
///     runner's owner: neither the token's own account nor an `owner` role.
///     A removed member's membership is deleted with the removal, so they are
///     refused at once.
///   - At most one push per runner and member every six hours, and an event id
///     already used sends nothing, so a retry after a timeout is free.
///
/// The words are the relay's: "Your Claude login on <Runner> stopped working."
/// with the runner's name from the relay's own record, never from the request.
/// No email, no text of the daemon's. Every app install of the account hears
/// it, whatever its notification settings, as a device-key notice does.
///
/// **Contract** (lane daemon-646 builds against this):
///
///     POST /v1/notify/member
///     authorization: Bearer <the runner's daemon token>
///     { "account": "<the member's relay account id, `members.account`>",
///       "kind": "login",
///       "event": "<the stoppage's id: [A-Za-z0-9:._-], 1-128 chars>",
///       "runner": "<optional: Host.runner_id, [A-Za-z0-9-], 1-64 chars>" }
///
///     200 {"ok":true}             taken: sent, or dropped for any reason below
///     400 {"error":"account"}     `account` missing or not an account id's shape
///     400 {"error":"kind"}        `kind` is not "login"
///     400 {"error":"event"}       `event` missing or the wrong shape
///     400 {"error":"body"}        not a JSON object
///     401 {"error":"unauthorized"} no token, or one the relay doesn't hold
///     500 {"error":"relay misconfigured", ...}  as `/v1/notify`
///
/// **One answer for every refusal.** A non-member, the owner, a billing
/// member, an unknown account, an unregistered runner, a throttled member and
/// a repeated event all get the same 200 `{"ok":true}`, byte for byte, as a
/// push that went out. Otherwise a token holder could ask the route who is in
/// the team. The 400s and 401 depend only on the request's shape and the
/// token, never on who the account is. The membership check and the throttle
/// are one statement whether the account is a member or not, and the pushes go
/// out after the response (`waitUntil`), so the time taken says no more than
/// the body does.
///
/// `runner` is forwarded, never shown: it is what a tap opens, the same value
/// `/v1/notify` forwards. A daemon should treat any 200 as done and log a 404
/// (a relay older than this route) and move on.

import { json, requireDaemon, type Env } from './index'
import { audit, withoutUnshown } from './directory/common'
import { tellAccount } from './directory/notices'
import { topicMismatch } from './push'

/// How long after one push a runner may push the same member again.
export const MEMBER_NOTICE_EVERY_MS = 6 * 60 * 60 * 1000

/// How long a sent notice's row is kept. Past this an event id could be used
/// again, which only an honest retry would ever do, and a retry is minutes late,
/// not a month.
export const MEMBER_NOTICE_KEPT_MS = 30 * 24 * 60 * 60 * 1000

/// A relay account id: a WorkOS user id.
const ACCOUNT = /^[A-Za-z0-9_-]{1,128}$/
const EVENT = /^[A-Za-z0-9:._-]{1,128}$/
const RUNNER_ID = /^[A-Za-z0-9-]{1,64}$/

/// The push a member gets when their Claude login on `runner` stops working.
/// The runner's name as stored, without anything a notice mustn't show, cut to
/// 40 characters.
export function loginNotice(runner: string): { title: string; subtitle: string } {
  const plain = withoutUnshown(runner).replace(/\s+/g, ' ').trim()
  const name = [...plain].slice(0, 40).join('').trim() || 'a runner'
  return {
    title: `Your Claude login on ${name} stopped working.`,
    subtitle: 'Sign in again in Far Cooler.',
  }
}

/// The one answer to every request the relay took, sent or not.
const TAKEN = () => json({ ok: true })

export async function notifyMember(request: Request, env: Env, ctx?: ExecutionContext): Promise<Response> {
  const daemon = await requireDaemon(request, env)
  if (daemon instanceof Response) return daemon

  let body: Record<string, unknown>
  try {
    const parsed = await request.json()
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return json({ error: 'body' }, 400)
    body = parsed as Record<string, unknown>
  } catch {
    return json({ error: 'body' }, 400)
  }
  const account = typeof body.account === 'string' && ACCOUNT.test(body.account) ? body.account : null
  if (!account) return json({ error: 'account' }, 400)
  if (body.kind !== 'login') return json({ error: 'kind' }, 400)
  const kind = 'login'
  const event = typeof body.event === 'string' && EVENT.test(body.event) ? body.event : null
  if (!event) return json({ error: 'event' }, 400)
  const phoneRunner = typeof body.runner === 'string' && RUNNER_ID.test(body.runner) ? body.runner.toLowerCase() : undefined

  const misconfigured = topicMismatch(env)
  if (misconfigured) {
    console.error(`relay misconfigured: ${misconfigured}`)
    return json({ error: 'relay misconfigured', detail: misconfigured }, 500)
  }

  // Which runner this token is. None: a token paired the old way, which names
  // no team and so nobody in one.
  const runner = await env.DB.prepare(
    `SELECT id, team_id, name FROM runners WHERE daemon_id = ? ORDER BY created_at, id LIMIT 1`,
  )
    .bind(daemon.id)
    .first<{ id: string; team_id: string; name: string }>()
  if (!runner) return TAKEN()

  // The check and the throttle, as the one statement that records the push:
  // a row is written only for a member who may be named and hasn't been told
  // within six hours, and the audit line only with it. An event id already
  // used is the primary key's conflict, so it writes nothing either.
  const now = Date.now()
  const claim = env.DB.prepare(
    `INSERT INTO member_notices (runner_id, account_id, kind, event, sent_at)
     SELECT ?1, m.account_id, ?2, ?3, ?4
     FROM memberships m JOIN teams t ON t.id = m.team_id
     WHERE m.team_id = ?5 AND m.account_id = ?6 AND m.role IN ('admin', 'member')
       AND t.personal_of IS NULL
       AND m.account_id != ?7
       AND NOT EXISTS (SELECT 1 FROM runner_roles o
                       WHERE o.runner_id = ?1 AND o.account_id = m.account_id AND o.role = 'owner')
       AND NOT EXISTS (SELECT 1 FROM member_notices n
                       WHERE n.runner_id = ?1 AND n.account_id = m.account_id
                         AND n.sent_at > ?8)
     ON CONFLICT DO NOTHING`,
  ).bind(runner.id, kind, event, now, runner.team_id, account, daemon.account_id, now - MEMBER_NOTICE_EVERY_MS)
  const line = audit(
    env,
    { team: runner.team_id, actor: `runner:${runner.id}`, action: 'runner.member_notified', subject: account, detail: { runner: runner.id, kind } },
    {
      sql: `EXISTS (SELECT 1 FROM member_notices WHERE runner_id = ? AND account_id = ? AND event = ? AND sent_at = ?)`,
      binds: [runner.id, account, event, now],
    },
  )
  const prune = env.DB.prepare(`DELETE FROM member_notices WHERE runner_id = ? AND account_id = ? AND sent_at < ?`)
    .bind(runner.id, account, now - MEMBER_NOTICE_KEPT_MS)
  const [claimed] = await env.DB.batch([claim, line, prune])
  if ((claimed.meta.changes ?? 0) === 1) {
    const sending = tellAccount(env, account, loginNotice(runner.name), { kind, runner: phoneRunner })
    if (ctx) ctx.waitUntil(sending)
    else await sending
  }
  return TAKEN()
}
