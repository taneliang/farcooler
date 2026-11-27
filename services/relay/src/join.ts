/// `farcooler join` (ov-1490): a runner joins a person's account from its own
/// terminal, with no app.
///
/// Four routes, all POST:
///
/// - `/v1/join/ticket` (session), `{ceremony}`: the signed-in browser that is
///   showing the command asks for a one-use ticket for its offer's ceremony.
///   Answers `{ticket, team, expiresAt}`.
/// - `/v1/join/check` (ticket), `{ticket}`: whether the ticket can still take
///   a reply, so an expired command adds nothing on the runner.
/// - `/v1/join/reply` (ticket), `{ticket, sealed}`: the runner's sealed reply,
///   once. It joins the ceremony's replies as the account's own, since the
///   account asked for it, and the browser still opens it with its own key and
///   shows a code to compare. Answers `{claim}`.
/// - `/v1/join/approve` (session), `{ceremony}`: the person matched the codes.
/// - `/v1/join/register` (claim), `{claim, install, name, runnerKey,
///   issuedAt, proof}`: once approved, the runner is added to the account on
///   its own signature, exactly as `directory/runners/register` adds one.
///   Answers `{runner, team, token}`, once; before approval, 409 `waiting`.
///
/// **What a ticket is worth.** It grants no access to any runner: the browser's
/// key travels in the command the person copies, never through here, so this
/// relay can't put another key in its place. Someone who saw the command could
/// post a reply first; it opens, and shows a code that doesn't match the one in
/// the person's terminal. A runner reaches the account only after a signed-in
/// approval, and only for whoever posted the reply that was approved.

import { REPLY_TTL_MS, looksSealed, SEALED } from './ceremony-reply'
import { audit, bodyOf, requireUser } from './directory/common'
import { register, proveRunner } from './directory/runners'
import { readLimited } from './heartbeat-body'
import { json, sha256, type Env } from './index'

export const JOIN_PREFIX = '/v1/join/'

/// How long a ticket takes a reply: the ceremony's window, which is all the
/// browser will wait for one.
export const TICKET_TTL_MS = REPLY_TTL_MS

/// How long after a reply the person has to compare the codes and the runner
/// to collect its token.
export const APPROVAL_TTL_MS = 3 * 60 * 1000

/// How many tickets one account holds at once; the oldest goes first.
export const TICKETS_PER_ACCOUNT = 8

const CEREMONY = /^[0-9a-f]{32}$/
const SECRET = /^[0-9a-f]{64}$/
const BODY_MAX = 16 * 1024

const hex = () => [...crypto.getRandomValues(new Uint8Array(32))].map(b => b.toString(16).padStart(2, '0')).join('')
const hashed = (kind: 'ticket' | 'claim', secret: string) => sha256(`join-${kind}:${secret}`)
/// One answer for every ticket or claim that isn't live, so a guess learns nothing.
const gone = () => json({ error: 'not found' }, 404)

export async function join(request: Request, env: Env, pathname: string): Promise<Response> {
  switch (pathname.slice(JOIN_PREFIX.length)) {
    case 'ticket':
      return await ticket(request, env)
    case 'approve':
      return await approve(request, env)
    case 'check':
      return await throttled(request, env, check)
    case 'reply':
      return await throttled(request, env, reply)
    case 'register':
      return await throttled(request, env, finish)
    default:
      return json({ error: 'not found' }, 404)
  }
}

/// The routes a runner calls with no session, throttled by address before they
/// read anything. The ceremony routes' limit, under a key of its own: a
/// waiting `farcooler join` asks 20 times a minute.
async function throttled(request: Request, env: Env, route: (body: Record<string, unknown>, request: Request, env: Env) => Promise<Response>): Promise<Response> {
  const address = request.headers.get('cf-connecting-ip') ?? 'unknown'
  if (env.CEREMONY_REPLY_LIMIT && !(await env.CEREMONY_REPLY_LIMIT.limit({ key: `join:${address}` })).success) {
    return json({ error: 'slow down' }, 429)
  }
  const text = await readLimited(request, BODY_MAX)
  if (text === null) return json({ error: 'too large' }, 413)
  let body: unknown
  try {
    body = JSON.parse(text)
  } catch {
    return json({ error: 'body' }, 400)
  }
  if (!body || typeof body !== 'object' || Array.isArray(body)) return json({ error: 'body' }, 400)
  return await route(body as Record<string, unknown>, request, env)
}

async function ticket(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env, env.CEREMONY_REPLY_LIMIT)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const ceremony = body?.ceremony
  if (typeof ceremony !== 'string' || !CEREMONY.test(ceremony)) return json({ error: 'ceremony' }, 400)
  const team = await env.DB.prepare(`SELECT id FROM teams WHERE personal_of = ?`).bind(account).first<{ id: string }>()
  if (!team) return gone()
  const secret = hex()
  const now = Date.now()
  const expiresAt = now + TICKET_TTL_MS
  await env.DB.batch([
    env.DB.prepare(`DELETE FROM join_tickets WHERE expires_at <= ?`).bind(now),
    env.DB.prepare(
      `DELETE FROM join_tickets WHERE account_id = ?1 AND ticket_hash IN (
         SELECT ticket_hash FROM join_tickets WHERE account_id = ?1
         ORDER BY created_at DESC, ticket_hash LIMIT -1 OFFSET ?2)`,
    ).bind(account, TICKETS_PER_ACCOUNT - 1),
    env.DB.prepare(
      `INSERT INTO join_tickets (ticket_hash, account_id, team_id, ceremony, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)`,
    ).bind(await hashed('ticket', secret), account, team.id, ceremony, now, expiresAt),
  ])
  return json({ ticket: secret, team: team.id, expiresAt })
}

async function check(body: Record<string, unknown>, _request: Request, env: Env): Promise<Response> {
  if (typeof body.ticket !== 'string' || !SECRET.test(body.ticket)) return gone()
  const live = await env.DB.prepare(`SELECT 1 FROM join_tickets WHERE ticket_hash = ? AND claim_hash IS NULL AND expires_at > ?`)
    .bind(await hashed('ticket', body.ticket), Date.now())
    .first()
  return live ? json({ ok: true }) : gone()
}

async function reply(body: Record<string, unknown>, _request: Request, env: Env): Promise<Response> {
  const { ticket, sealed } = body
  if (typeof ticket !== 'string' || !SECRET.test(ticket)) return gone()
  if (typeof sealed !== 'string' || !SEALED.test(sealed) || !looksSealed(sealed)) return json({ error: 'sealed' }, 400)
  const claim = hex()
  const claimHash = await hashed('claim', claim)
  const now = Date.now()
  // One batch: the UPDATE takes the ticket or nothing does, and the reply is
  // written only for the request that took it.
  const [taken] = await env.DB.batch([
    env.DB.prepare(
      `UPDATE join_tickets SET claim_hash = ?1, expires_at = ?2 WHERE ticket_hash = ?3 AND claim_hash IS NULL AND expires_at > ?4`,
    ).bind(claimHash, now + APPROVAL_TTL_MS, await hashed('ticket', ticket), now),
    env.DB.prepare(
      `INSERT INTO ceremony_replies (ceremony, account_id, posted_by, sealed, created_at)
       SELECT ceremony, account_id, account_id, ?1, ?2 FROM join_tickets WHERE claim_hash = ?3
       ON CONFLICT (ceremony, posted_by) DO UPDATE SET sealed = ?1, created_at = ?2`,
    ).bind(sealed, now, claimHash),
  ])
  if ((taken.meta?.changes ?? 0) !== 1) return gone()
  return json({ claim })
}

async function approve(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env, env.CEREMONY_REPLY_LIMIT)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const ceremony = body?.ceremony
  if (typeof ceremony !== 'string' || !CEREMONY.test(ceremony)) return json({ error: 'ceremony' }, 400)
  const now = Date.now()
  const row = await env.DB.prepare(
    `SELECT ticket_hash, team_id FROM join_tickets
     WHERE account_id = ? AND ceremony = ? AND claim_hash IS NOT NULL AND approved_at IS NULL AND expires_at > ?`,
  )
    .bind(account, ceremony, now)
    .first<{ ticket_hash: string; team_id: string }>()
  if (!row) return gone()
  await env.DB.batch([
    env.DB.prepare(`UPDATE join_tickets SET approved_at = ?1 WHERE ticket_hash = ?2 AND approved_at IS NULL`).bind(now, row.ticket_hash),
    audit(
      env,
      { team: row.team_id, actor: account, action: 'runner.join_approved', subject: ceremony },
      { sql: `EXISTS (SELECT 1 FROM join_tickets WHERE ticket_hash = ? AND approved_at = ?)`, binds: [row.ticket_hash, now] },
    ),
  ])
  return json({ ok: true })
}

async function finish(body: Record<string, unknown>, request: Request, env: Env): Promise<Response> {
  if (typeof body.claim !== 'string' || !SECRET.test(body.claim)) return gone()
  const claimHash = await hashed('claim', body.claim)
  const row = await env.DB.prepare(`SELECT account_id, team_id, approved_at FROM join_tickets WHERE claim_hash = ? AND expires_at > ?`)
    .bind(claimHash, Date.now())
    .first<{ account_id: string; team_id: string; approved_at: number | null }>()
  if (!row) return gone()
  if (row.approved_at === null) return json({ error: 'waiting' }, 409)
  const proven = await proveRunner(request, env, 'register', row.account_id, { ...body, team: row.team_id })
  if (proven instanceof Response) return proven
  // Spent by one DELETE whose change count decides, once the proof is good, so
  // a claim adds one runner and a stale proof doesn't use it up.
  const spent = await env.DB.prepare(`DELETE FROM join_tickets WHERE claim_hash = ? AND approved_at IS NOT NULL`).bind(claimHash).run()
  if ((spent.meta?.changes ?? 0) !== 1) return gone()
  return await register(env, proven)
}
