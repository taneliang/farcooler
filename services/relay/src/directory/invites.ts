/// Invite links: how people join a team (ov-480 §4).
///
/// A team admin makes a link with an expiry, a use limit and optionally an
/// email domain. Its code is shown once and kept only as a hash. Anyone signed
/// in with the link joins as the role it carries, `member` or `billing`, never
/// admin. Standing grants carried on a link are phase 5.

import { json, sha256, type Env } from '../index'
import { audit, bodyOf, requireUser, teamFor } from './common'

const DAY_MS = 24 * 60 * 60 * 1000
const MAX_DAYS = 30
const MAX_USES = 500
const DOMAIN = /^(?=.{1,253}$)([a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/

export async function createInvite(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, ['admin'])
  if (found instanceof Response) return found
  if (found.team.personal_of !== null) return json({ error: 'personal' }, 409)

  const role = body?.role ?? 'member'
  if (role !== 'member' && role !== 'billing') return json({ error: 'role' }, 400)
  const days = body?.days ?? 7
  if (typeof days !== 'number' || !Number.isInteger(days) || days < 1 || days > MAX_DAYS) return json({ error: 'days' }, 400)
  const uses = body?.uses ?? 1
  if (typeof uses !== 'number' || !Number.isInteger(uses) || uses < 1 || uses > MAX_USES) return json({ error: 'uses' }, 400)
  let domain: string | null = null
  if (body?.domain !== undefined && body.domain !== null) {
    if (typeof body.domain !== 'string' || !DOMAIN.test(body.domain.toLowerCase())) return json({ error: 'domain' }, 400)
    domain = body.domain.toLowerCase()
  }

  const code = crypto.randomUUID().replaceAll('-', '') + crypto.randomUUID().replaceAll('-', '')
  const id = crypto.randomUUID()
  const now = Date.now()
  const expiresAt = now + days * DAY_MS
  await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO invites (id, team_id, code_hash, role, domain, max_uses, created_by, created_at, expires_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    ).bind(id, found.team.id, await sha256(code), role, domain, uses, account, now, expiresAt),
    audit(env, { team: found.team.id, actor: account, action: 'invite.created', subject: id, detail: { role, domain, uses, days } }),
  ])
  return json({ invite: id, code, expiresAt })
}

export async function listInvites(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, ['admin'])
  if (found instanceof Response) return found

  const rows = await env.DB.prepare(
    `SELECT id, role, domain, max_uses, uses, created_by, created_at, expires_at, revoked_at FROM invites
     WHERE team_id = ? ORDER BY created_at DESC`,
  )
    .bind(found.team.id)
    .all<Record<string, unknown>>()
  return json({
    invites: rows.results.map(r => ({
      id: r.id,
      role: r.role,
      domain: r.domain,
      maxUses: r.max_uses,
      uses: r.uses,
      createdBy: r.created_by,
      createdAt: r.created_at,
      expiresAt: r.expires_at,
      revokedAt: r.revoked_at,
    })),
  })
}

export async function revokeInvite(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, ['admin'])
  if (found instanceof Response) return found
  if (typeof body?.invite !== 'string' || !body.invite) return json({ error: 'invite' }, 400)

  const now = Date.now()
  const [changed] = await env.DB.batch([
    env.DB.prepare(`UPDATE invites SET revoked_at = ? WHERE id = ? AND team_id = ? AND revoked_at IS NULL`).bind(
      now,
      body.invite,
      found.team.id,
    ),
    audit(
      env,
      { team: found.team.id, actor: account, action: 'invite.revoked', subject: body.invite },
      // Only if this request revoked it: an invite already revoked logs nothing.
      { sql: `EXISTS (SELECT 1 FROM invites WHERE id = ? AND team_id = ? AND revoked_at = ?)`, binds: [body.invite, found.team.id, now] },
    ),
  ])
  return json({ ok: (changed.meta?.changes ?? 0) > 0 })
}

/// Join a team with an invite code.
///
/// A code that doesn't exist, or that no longer works (revoked, expired, used
/// up), gets a 404, and so does a person an admin removed from the team: a
/// removal can't be undone with a link, only by an admin re-adding them
/// (`teams.ts`). A domain-limited code answers 403 to anyone whose
/// WorkOS-verified email is elsewhere, before its other limits are looked at;
/// with 244-bit codes that says nothing a guess could use. Someone already in
/// the team is told so and spends nothing.
///
/// The join is one transaction: the membership goes in only if the invite
/// still works and the person isn't barred, and a use is spent, and the audit
/// line written, only if this request's own row went in (`join_nonce`). So
/// the last use can't be spent twice, a failure can't spend a use without a
/// join, and the same person accepting twice at once spends one use.
export async function acceptInvite(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (typeof body?.code !== 'string' || !/^[0-9a-f]{64}$/.test(body.code)) return json({ error: 'not found' }, 404)
  const now = Date.now()

  const invite = await env.DB.prepare(
    `SELECT i.id, i.team_id, i.role, i.domain, t.name FROM invites i JOIN teams t ON t.id = i.team_id
     WHERE i.code_hash = ?`,
  )
    .bind(await sha256(body.code))
    .first<{ id: string; team_id: string; role: string; domain: string | null; name: string }>()
  if (!invite) return json({ error: 'not found', reason: 'unknown' }, 404)

  if (invite.domain) {
    // Never the access token's word, and never an unverified address: see
    // `verified_email` in migration 0021.
    const who = await env.DB.prepare(`SELECT verified_email FROM accounts WHERE id = ?`)
      .bind(account)
      .first<{ verified_email: string | null }>()
    if (who?.verified_email?.split('@').pop() !== invite.domain) return json({ error: 'domain' }, 403)
  }

  const team = { id: invite.team_id, name: invite.name }
  const already = await env.DB.prepare(`SELECT role FROM memberships WHERE team_id = ? AND account_id = ?`)
    .bind(invite.team_id, account)
    .first<{ role: string }>()
  if (already) return json({ team, role: already.role, joined: false })

  const nonce = crypto.randomUUID()
  const joined = `EXISTS (SELECT 1 FROM memberships WHERE team_id = ? AND account_id = ? AND join_nonce = ?)`
  const [inserted] = await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO memberships (team_id, account_id, role, joined_at, join_nonce)
       SELECT ?1, ?2, ?3, ?4, ?5
       WHERE EXISTS (SELECT 1 FROM invites WHERE id = ?6 AND revoked_at IS NULL AND expires_at > ?4 AND uses < max_uses)
         AND NOT EXISTS (SELECT 1 FROM team_removals WHERE team_id = ?1 AND account_id = ?2)
       ON CONFLICT (team_id, account_id) DO NOTHING`,
    ).bind(invite.team_id, account, invite.role, now, nonce, invite.id),
    env.DB.prepare(`UPDATE invites SET uses = uses + 1 WHERE id = ? AND ${joined}`).bind(invite.id, invite.team_id, account, nonce),
    audit(
      env,
      { team: invite.team_id, actor: account, action: 'member.joined', subject: account, detail: { invite: invite.id, role: invite.role } },
      { sql: joined, binds: [invite.team_id, account, nonce] },
    ),
  ])
  if ((inserted.meta?.changes ?? 0) > 0) return json({ team, role: invite.role, joined: true })

  // Joined by a request racing this one, or not let in at all.
  const raced = await env.DB.prepare(`SELECT role FROM memberships WHERE team_id = ? AND account_id = ?`)
    .bind(invite.team_id, account)
    .first<{ role: string }>()
  if (raced) return json({ team, role: raced.role, joined: false })
  return json({ error: 'not found', reason: await whyNot(env, invite.id, invite.team_id, account, now) }, 404)
}

/// Why a code that exists doesn't let this person in, for the sentence a
/// person is shown. The answer only goes to someone holding the 244-bit code,
/// so it says nothing a guess could use. A removal is checked first: it
/// outlasts the invite's own limits.
async function whyNot(env: Env, invite: string, team: string, account: string, now: number): Promise<string> {
  const [removed, row] = await env.DB.batch([
    env.DB.prepare(`SELECT 1 AS found FROM team_removals WHERE team_id = ? AND account_id = ?`).bind(team, account),
    env.DB.prepare(`SELECT revoked_at, expires_at, uses, max_uses FROM invites WHERE id = ?`).bind(invite),
  ])
  if (removed.results.length > 0) return 'removed'
  const i = row.results[0] as { revoked_at: number | null; expires_at: number; uses: number; max_uses: number } | undefined
  if (!i) return 'unknown'
  if (i.revoked_at !== null) return 'revoked'
  if (i.expires_at <= now) return 'expired'
  if (i.uses >= i.max_uses) return 'used'
  return 'unknown'
}

/// What a code would do for this person, without spending it: the team's
/// name and the role it carries, or the same refusals `acceptInvite` gives,
/// with the same reasons. It lets the web ask "Join Acme?" before anyone
/// joins. Someone already in the team is told so.
export async function previewInvite(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (typeof body?.code !== 'string' || !/^[0-9a-f]{64}$/.test(body.code)) return json({ error: 'not found', reason: 'unknown' }, 404)
  const now = Date.now()

  const invite = await env.DB.prepare(
    `SELECT i.id, i.team_id, i.role, i.domain, t.name, a.verified_email AS creator FROM invites i JOIN teams t ON t.id = i.team_id
     LEFT JOIN accounts a ON a.id = i.created_by
     WHERE i.code_hash = ?`,
  )
    .bind(await sha256(body.code))
    .first<{ id: string; team_id: string; role: string; domain: string | null; name: string; creator: string | null }>()
  if (!invite) return json({ error: 'not found', reason: 'unknown' }, 404)
  if (invite.domain) {
    const who = await env.DB.prepare(`SELECT verified_email FROM accounts WHERE id = ?`)
      .bind(account)
      .first<{ verified_email: string | null }>()
    if (who?.verified_email?.split('@').pop() !== invite.domain) return json({ error: 'domain' }, 403)
  }
  const team = { id: invite.team_id, name: invite.name }
  const already = await env.DB.prepare(`SELECT role FROM memberships WHERE team_id = ? AND account_id = ?`)
    .bind(invite.team_id, account)
    .first<{ role: string }>()
  if (already) return json({ team, role: already.role, member: true })

  const reason = await whyNot(env, invite.id, invite.team_id, account, now)
  if (reason !== 'unknown') return json({ error: 'not found', reason }, 404)
  // Who made the link, by their verified email only: an address nobody checked
  // must never read as "Invited by".
  return json({ team, role: invite.role, member: false, invitedBy: invite.creator })
}
