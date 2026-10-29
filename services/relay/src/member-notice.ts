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
///   - At most one push per runner, member and kind in that kind's window (six
///     hours for a login, two minutes for Needs You), and an event id already
///     used sends nothing, so a retry after a timeout is free.
///
/// The words are the relay's, one fixed template per kind, with the runner's
/// name from the relay's own record, never from the request:
///
///   - `login`: "Your Claude login on <Runner> stopped working."
///   - `approval` (ov-858): "Approve a change on <Runner>." Sent when a member's
///     Mac, which can't hold a step-up key, asks their phone to approve an admin
///     action. It never names the action, goes to every install of the account
///     whatever its settings (only the person can approve), at most once in ten
///     seconds, and a tap opens the app, which lists what is waiting.
///   - `needs_you` (ov-633): "Something on <Runner> needs you." Sent when an
///     agent, a decision or a review that is that member's (they started the
///     agent, or it is their rep's or lane's) is waiting on a person; the
///     owner's phone moves its card for it without a banner once this route
///     has answered 200. It carries `class` (`decision`, `blocked` or
///     `review`, `blocked` when absent), and goes only to the member's
///     installs that keep that class on, as the owner's task notices do. A
///     login push ignores those settings (ov-646): only its member can fix it.
///
/// The runner's name in the words is the owner's choice at registration, so
/// a hostile owner controls up to 40 characters of a push to their own team's
/// members: once per six hours for a login, and up to 30 an hour for Needs
/// You. Accepted: it is their own team, and the rest of the words are fixed.
///
/// Only a login push writes a `runner.member_notified` audit line (ov-633
/// review L2): a Needs You push every two minutes would bury the team's log.
/// No email, no text of the daemon's. A login push reaches every app install
/// of the account, whatever its notification settings, as a device-key notice
/// does; a Needs You push only the installs that keep its class on.
///
/// **`sent`, on a Needs You push only** (ov-633): whether the member has been
/// pushed, now or within the window (a throttled push answers true, because
/// the slot is only ever taken by a push that went). False for every refusal,
/// and for a member with no install, or none keeping the class on, for which
/// no slot is taken, so the next push isn't held back. The runner quiets the
/// owner's own notice only on true, so a push nobody got leaves the owner
/// buzzed. It tells the token holder whether an account is in the team with a
/// phone, which the runner already knows: it is the owner's own, and it named
/// the member from its own records (`members.account`). A login push answers
/// `{"ok":true}` as before.
///
/// **Contract** (lane daemon-646 builds against this):
///
///     POST /v1/notify/member
///     authorization: Bearer <the runner's daemon token>
///     { "account": "<the member's relay account id, `members.account`>",
///       "kind": "login" | "needs_you" | "approval",
///       "event": "<the stoppage's id: [A-Za-z0-9:._-], 1-128 chars>",
///       "class": "<needs_you only, optional: decision | blocked | review>",
///       "runner": "<optional: Host.runner_id, [A-Za-z0-9-], 1-64 chars>",
///       "task": "<needs_you only, optional: the task's key, [A-Za-z0-9._-], 1-64 chars>",
///       "terminal": "<needs_you only, optional: the pane's id, [A-Za-z0-9-], 1-64 chars>" }
///
///     200 {"ok":true}             taken: sent, or dropped for any reason below
///     200 {"ok":true,"sent":b}    the same, for needs_you: see `sent` above
///     400 {"error":"account"}     `account` missing or not an account id's shape
///     400 {"error":"kind"}        `kind` is none of "login", "needs_you" and "approval"
///     400 {"error":"event"}       `event` missing or the wrong shape
///     400 {"error":"class"}       `class` present and not one of the three
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
/// `/v1/notify` forwards. It is forwarded unchecked against any record, as
/// `/v1/notify` does, because the relay doesn't store raw install ids.
///
/// **What a Needs You tap opens** (ov-705): `task` (a key, with the class as
/// `event`, so a decision opens at its question) or else `terminal` (a pane's
/// id), forwarded the same way and never shown. Both are ids the member's own
/// app already lists in Needs You, and the owner's `/v1/notify` forwards the
/// same two. One of the wrong shape is dropped, not refused: the push still
/// goes and a tap opens Needs You, as it does when neither is sent.
///
/// A body over `MEMBER_NOTICE_BODY_MAX` bytes is the 400 `{"error":"body"}`,
/// refused without reading past the limit. A daemon should treat any 200 as done and log a 404
/// (a relay older than this route) and move on.

import { json, requireDaemon, type Env } from './index'
import { keepsEvent } from './event-classes'
import { audit, withoutUnshown } from './directory/common'
import { tellAccount } from './directory/notices'
import { readLimited } from './heartbeat-body'
import { topicMismatch } from './push'

/// How long after one login push a runner may push the same member again.
export const MEMBER_NOTICE_EVERY_MS = 6 * 60 * 60 * 1000

/// How long after one Needs You push a runner may push the same member again
/// (ov-633): a burst of asks is one buzz, and a stolen token can buzz a team
/// member at most this often, with fixed words.
export const NEEDS_YOU_NOTICE_EVERY_MS = 2 * 60 * 1000

/// How long after one approval push a runner may push the same member again
/// (ov-858): an approval waits two minutes at most, so a Mac asking again
/// inside the window is the same buzz.
export const APPROVAL_NOTICE_EVERY_MS = 10 * 1000

/// Each kind a runner may push a member about, and its window.
const KINDS: Record<string, number> = {
  login: MEMBER_NOTICE_EVERY_MS,
  needs_you: NEEDS_YOU_NOTICE_EVERY_MS,
  approval: APPROVAL_NOTICE_EVERY_MS,
}

/// How long a sent notice's row is kept. Past this an event id could be used
/// again, which only an honest retry would ever do, and a retry is minutes late,
/// not a month.
export const MEMBER_NOTICE_KEPT_MS = 30 * 24 * 60 * 60 * 1000

/// The largest body the route reads. A real one is four short strings.
export const MEMBER_NOTICE_BODY_MAX = 4 * 1024

/// A relay account id: a WorkOS user id.
const ACCOUNT = /^[A-Za-z0-9_-]{1,128}$/
const EVENT = /^[A-Za-z0-9:._-]{1,128}$/
const RUNNER_ID = /^[A-Za-z0-9-]{1,64}$/
/// A task's key (`bil-7`), and a pane's id (a UUID), as a Needs You tap opens them.
const TASK_KEY = /^[A-Za-z0-9._-]{1,64}$/
const TERMINAL_ID = /^[A-Za-z0-9-]{1,64}$/
/// A Needs You push's classes: a device's switches it answers to.
const NEEDS_YOU_CLASSES = ['decision', 'blocked', 'review']

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

/// The push a member gets when something of theirs on `runner` is waiting on
/// them (ov-633). The runner's name as `loginNotice` cuts it.
export function needsYouNotice(runner: string): { title: string; subtitle: string } {
  const plain = withoutUnshown(runner).replace(/\s+/g, ' ').trim()
  const name = [...plain].slice(0, 40).join('').trim() || 'a runner'
  return { title: `Something on ${name} needs you.`, subtitle: 'See it in Needs You.' }
}

/// The push a member gets when a Mac of theirs is waiting for their phone to
/// approve an admin action on `runner` (ov-858). The words never name the
/// action: the phone shows it, behind Face ID. The runner's name as
/// `loginNotice` cuts it.
export function approvalNotice(runner: string): { title: string; subtitle: string } {
  const plain = withoutUnshown(runner).replace(/\s+/g, ' ').trim()
  const name = [...plain].slice(0, 40).join('').trim() || 'a runner'
  return { title: `Approve a change on ${name}.`, subtitle: 'Open Far Cooler to review it.' }
}

/// The one answer to every request the relay took, sent or not.
const TAKEN = () => json({ ok: true })

export async function notifyMember(request: Request, env: Env, ctx?: ExecutionContext): Promise<Response> {
  const daemon = await requireDaemon(request, env)
  if (daemon instanceof Response) return daemon

  let body: Record<string, unknown>
  const text = await readLimited(request, MEMBER_NOTICE_BODY_MAX)
  if (text === null) return json({ error: 'body' }, 400)
  try {
    const parsed = JSON.parse(text)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return json({ error: 'body' }, 400)
    body = parsed as Record<string, unknown>
  } catch {
    return json({ error: 'body' }, 400)
  }
  const account = typeof body.account === 'string' && ACCOUNT.test(body.account) ? body.account : null
  if (!account) return json({ error: 'account' }, 400)
  if (typeof body.kind !== 'string' || !Object.hasOwn(KINDS, body.kind)) return json({ error: 'kind' }, 400)
  const kind = body.kind as 'login' | 'needs_you' | 'approval'
  const every = KINDS[kind]
  const event = typeof body.event === 'string' && EVENT.test(body.event) ? body.event : null
  if (!event) return json({ error: 'event' }, 400)
  if (body.class !== undefined && (typeof body.class !== 'string' || !NEEDS_YOU_CLASSES.includes(body.class))) {
    return json({ error: 'class' }, 400)
  }
  const needsClass = typeof body.class === 'string' ? body.class : 'blocked'
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
  if (!runner) return kind === 'needs_you' ? json({ ok: true, sent: false }) : TAKEN()

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
                         AND n.kind = ?2 AND n.sent_at > ?8)
     ON CONFLICT DO NOTHING`,
  ).bind(runner.id, kind, event, now, runner.team_id, account, daemon.account_id, now - every)
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
  if (kind === 'needs_you') {
    // The claim before the push, so two at once can't both go; given back if
    // nothing went, so a push no install wanted holds back no later one.
    const [claimed] = await env.DB.batch([claim, prune])
    if ((claimed.meta.changes ?? 0) !== 1) return json({ ok: true, sent: await toldWithin(env, runner.id, account, now - every) })
    const task = typeof body.task === 'string' && TASK_KEY.test(body.task) ? body.task : undefined
    const terminal = !task && typeof body.terminal === 'string' && TERMINAL_ID.test(body.terminal) ? body.terminal : undefined
    const opens = task ? { task, event: needsClass } : terminal ? { terminal } : {}
    const sent = await tellAccount(env, account, needsYouNotice(runner.name), { kind, runner: phoneRunner, ...opens }, saved =>
      keepsEvent(saved, needsClass),
    )
    if (!sent) {
      await env.DB.prepare(`DELETE FROM member_notices WHERE runner_id = ? AND account_id = ? AND event = ?`)
        .bind(runner.id, account, event)
        .run()
    }
    return json({ ok: true, sent })
  }
  const [claimed] = await env.DB.batch([claim, line, prune])
  if ((claimed.meta.changes ?? 0) === 1) {
    const words = kind === 'approval' ? approvalNotice(runner.name) : loginNotice(runner.name)
    const sending = tellAccount(env, account, words, { kind, runner: phoneRunner })
    if (ctx) ctx.waitUntil(sending)
    else await sending
  }
  return TAKEN()
}

/// Whether `account` was sent a Needs You push from `runner` since `since`:
/// a claim that wrote nothing was throttled (they were), a repeated event
/// (they were, if it went), or a refusal (they weren't, and have no row).
async function toldWithin(env: Env, runner: string, account: string, since: number): Promise<boolean> {
  const row = await env.DB.prepare(
    `SELECT 1 AS told FROM member_notices WHERE runner_id = ? AND account_id = ? AND kind = 'needs_you' AND sent_at > ? LIMIT 1`,
  )
    .bind(runner, account, since)
    .first<{ told: number }>()
  return row !== null
}
