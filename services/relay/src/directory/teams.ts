/// Teams and the people in them.
///
/// Team admins add people (by invite, `invites.ts`), change their roles and
/// remove them. A team always keeps at least one admin, checked inside the
/// write rather than before it, so two admins removing each other at once
/// can't leave a team with none. A personal team has one person, its admin,
/// for good.

import { json, type Env } from '../index'
import { audit, bodyOf, nameOf, requireUser, teamFor, TEAM_ROLES, type TeamRole } from './common'

/// Who the caller is: their teams with their role in each, and their devices.
export async function me(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account

  const [who, teams, devices] = await env.DB.batch([
    env.DB.prepare(`SELECT email FROM accounts WHERE id = ?`).bind(account),
    env.DB.prepare(
      `SELECT t.id, t.name, t.personal_of, m.role FROM memberships m JOIN teams t ON t.id = m.team_id
       WHERE m.account_id = ? ORDER BY t.personal_of IS NULL, t.name`,
    ).bind(account),
    env.DB.prepare(
      `SELECT id, label, key_a, key_a_fingerprint, key_b, key_b_fingerprint, node_key, state,
              certified_by, created_at, certified_at, revoked_at
       FROM device_keys WHERE account_id = ? ORDER BY created_at`,
    ).bind(account),
  ])
  const email = (who.results[0] as { email: string | null } | undefined)?.email ?? null
  return json({
    user: { id: account, email },
    teams: (teams.results as { id: string; name: string; personal_of: string | null; role: TeamRole }[]).map(t => ({
      id: t.id,
      name: t.name,
      personal: t.personal_of !== null,
      role: t.role,
    })),
    devices: (devices.results as Record<string, unknown>[]).map(d => ({
      id: d.id,
      label: d.label,
      keyA: d.key_a,
      keyAFingerprint: d.key_a_fingerprint,
      keyB: d.key_b,
      keyBFingerprint: d.key_b_fingerprint,
      nodeKey: d.node_key,
      state: d.state,
      certifiedBy: d.certified_by,
      createdAt: d.created_at,
      certifiedAt: d.certified_at,
      revokedAt: d.revoked_at,
    })),
  })
}

/// Make a team, with the caller its first admin.
export async function createTeam(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const name = nameOf(body?.name)
  if (!name) return json({ error: 'name' }, 400)

  const id = crypto.randomUUID()
  const now = Date.now()
  await env.DB.batch([
    env.DB.prepare(`INSERT INTO teams (id, name, created_by, created_at) VALUES (?, ?, ?, ?)`).bind(id, name, account, now),
    env.DB.prepare(`INSERT INTO memberships (team_id, account_id, role, joined_at) VALUES (?, ?, 'admin', ?)`).bind(
      id,
      account,
      now,
    ),
    audit(env, { team: id, actor: account, action: 'team.created', detail: { name } }),
  ])
  return json({ team: { id, name, personal: false, role: 'admin' } })
}

/// Everyone in a team, for anyone in it but a billing member.
export async function listMembers(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, ['admin', 'member'])
  if (found instanceof Response) return found

  const rows = await env.DB.prepare(
    `SELECT m.account_id, a.email, m.role, m.joined_at FROM memberships m
     JOIN accounts a ON a.id = m.account_id
     WHERE m.team_id = ? ORDER BY m.joined_at`,
  )
    .bind(found.team.id)
    .all<{ account_id: string; email: string | null; role: TeamRole; joined_at: number }>()
  return json({
    team: { id: found.team.id, name: found.team.name, personal: found.team.personal_of !== null },
    members: rows.results.map(m => ({ user: m.account_id, email: m.email, role: m.role, joinedAt: m.joined_at })),
  })
}

/// Change a member's role. Admins only, never on a personal team, and never
/// the team's last admin to anything else.
export async function setRole(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, ['admin'])
  if (found instanceof Response) return found
  if (found.team.personal_of !== null) return json({ error: 'personal' }, 409)
  const role = body?.role
  if (typeof role !== 'string' || !TEAM_ROLES.includes(role as TeamRole)) return json({ error: 'role' }, 400)
  if (typeof body?.user !== 'string' || !body.user) return json({ error: 'user' }, 400)
  const user = body.user

  const current = await env.DB.prepare(`SELECT role FROM memberships WHERE team_id = ? AND account_id = ?`)
    .bind(found.team.id, user)
    .first<{ role: TeamRole }>()
  if (!current) return json({ error: 'not found' }, 404)
  if (current.role === role) return json({ ok: true })

  // The guard is in the UPDATE, not in the read above, so two admins demoting
  // each other at once can't both get through. The audit line is written only
  // if the role it records is the one now stored.
  const [changed] = await env.DB.batch([
    env.DB.prepare(
      `UPDATE memberships SET role = ?1 WHERE team_id = ?2 AND account_id = ?3
       AND (role != 'admin' OR (SELECT COUNT(*) FROM memberships WHERE team_id = ?2 AND role = 'admin') > 1)`,
    ).bind(role, found.team.id, user),
    audit(
      env,
      { team: found.team.id, actor: account, action: 'member.role_changed', subject: user, detail: { from: current.role, role } },
      { sql: `EXISTS (SELECT 1 FROM memberships WHERE team_id = ? AND account_id = ? AND role = ?)`, binds: [found.team.id, user, role] },
    ),
  ])
  if ((changed.meta?.changes ?? 0) > 0) return json({ ok: true })
  return json({ error: 'last admin' }, 409)
}

/// Take someone out of a team: an admin removing anyone, or anyone leaving.
///
/// Their roles on the team's runners go with them, in the same transaction.
/// That is the directory's half of offboarding. Taking their keys off each
/// runner is phase 6, and the audit line lists the runners it will have to
/// reach.
export async function removeMember(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, TEAM_ROLES)
  if (found instanceof Response) return found
  if (typeof body?.user !== 'string' || !body.user) return json({ error: 'user' }, 400)
  const user = body.user
  if (user !== account && found.role !== 'admin') return json({ error: 'forbidden' }, 403)
  if (found.team.personal_of !== null) return json({ error: 'personal' }, 409)

  const team = found.team.id
  const member = await env.DB.prepare(`SELECT role FROM memberships WHERE team_id = ? AND account_id = ?`)
    .bind(team, user)
    .first<{ role: TeamRole }>()
  if (!member) return json({ error: 'not found' }, 404)

  const access = await env.DB.prepare(
    `SELECT r.runner_id, r.role FROM runner_roles r JOIN runners n ON n.id = r.runner_id
     WHERE n.team_id = ? AND r.account_id = ?`,
  )
    .bind(team, user)
    .all<{ runner_id: string; role: string }>()

  const [removed] = await env.DB.batch([
    env.DB.prepare(
      `DELETE FROM memberships WHERE team_id = ?1 AND account_id = ?2
       AND (role != 'admin' OR (SELECT COUNT(*) FROM memberships WHERE team_id = ?1 AND role = 'admin') > 1)`,
    ).bind(team, user),
    env.DB.prepare(
      `DELETE FROM runner_roles WHERE account_id = ?1
       AND runner_id IN (SELECT id FROM runners WHERE team_id = ?2)
       AND NOT EXISTS (SELECT 1 FROM memberships WHERE team_id = ?2 AND account_id = ?1)`,
    ).bind(user, team),
    audit(
      env,
      {
        team,
        actor: account,
        action: user === account ? 'member.left' : 'member.removed',
        subject: user,
        detail: { runners: access.results.map(r => ({ runner: r.runner_id, role: r.role })) },
      },
      { sql: `NOT EXISTS (SELECT 1 FROM memberships WHERE team_id = ? AND account_id = ?)`, binds: [team, user] },
    ),
  ])
  if ((removed.meta?.changes ?? 0) > 0) return json({ ok: true, runners: access.results.length })
  return json({ error: 'last admin' }, 409)
}

/// The team's audit log, newest first, for its admins. `before` pages back.
export async function listAudit(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, ['admin'])
  if (found instanceof Response) return found
  const before = typeof body?.before === 'number' && Number.isSafeInteger(body.before) ? body.before : Number.MAX_SAFE_INTEGER

  const rows = await env.DB.prepare(
    `SELECT id, actor, action, subject, detail, at FROM audit
     WHERE team_id = ? AND id < ? ORDER BY id DESC LIMIT 100`,
  )
    .bind(found.team.id, before)
    .all<{ id: number; actor: string | null; action: string; subject: string | null; detail: string | null; at: number }>()
  return json({
    entries: rows.results.map(r => ({
      id: r.id,
      actor: r.actor,
      action: r.action,
      subject: r.subject,
      detail: r.detail === null ? null : JSON.parse(r.detail),
      at: r.at,
    })),
  })
}
