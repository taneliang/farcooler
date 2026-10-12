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

  const [changed] = await env.DB.batch([
    env.DB.prepare(`UPDATE invites SET revoked_at = ? WHERE id = ? AND team_id = ? AND revoked_at IS NULL`).bind(
      Date.now(),
      body.invite,
      found.team.id,
    ),
    audit(
      env,
      { team: found.team.id, actor: account, action: 'invite.revoked', subject: body.invite },
      { sql: `EXISTS (SELECT 1 FROM invites WHERE id = ? AND team_id = ?)`, binds: [body.invite, found.team.id] },
    ),
  ])
  return json({ ok: (changed.meta?.changes ?? 0) > 0 })
}

/// Join a team with an invite code.
///
/// Every reason a code doesn't work gets the same 404, so a guess learns
/// nothing about which codes exist. Whether it still works (not revoked, not
/// expired, a use left) is checked in one place, the UPDATE that spends a use,
/// so two people redeeming the last use at once can't both get in. Someone
/// already in the team is told so and spends nothing.
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
  if (!invite) return json({ error: 'not found' }, 404)

  if (invite.domain) {
    const who = await env.DB.prepare(`SELECT email FROM accounts WHERE id = ?`).bind(account).first<{ email: string | null }>()
    const domain = who?.email?.toLowerCase().split('@').pop()
    if (domain !== invite.domain) return json({ error: 'domain' }, 403)
  }

  const team = { id: invite.team_id, name: invite.name }
  const already = await env.DB.prepare(`SELECT role FROM memberships WHERE team_id = ? AND account_id = ?`)
    .bind(invite.team_id, account)
    .first<{ role: string }>()
  if (already) return json({ team, role: already.role, joined: false })

  const spent = await env.DB.prepare(
    `UPDATE invites SET uses = uses + 1 WHERE id = ? AND revoked_at IS NULL AND expires_at > ? AND uses < max_uses`,
  )
    .bind(invite.id, now)
    .run()
  if ((spent.meta?.changes ?? 0) === 0) return json({ error: 'not found' }, 404)

  await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO memberships (team_id, account_id, role, joined_at) VALUES (?, ?, ?, ?)
       ON CONFLICT (team_id, account_id) DO NOTHING`,
    ).bind(invite.team_id, account, invite.role, now),
    audit(env, { team: invite.team_id, actor: account, action: 'member.joined', subject: account, detail: { invite: invite.id, role: invite.role } }),
  ])
  return json({ team, role: invite.role, joined: true })
}
