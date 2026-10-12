/// What every directory route shares: who is asking, what they are in a team,
/// and the audit log.

import { json, requireAccount, type Env } from '../index'

export type TeamRole = 'admin' | 'member' | 'billing'
export type RunnerRole = 'owner' | 'admin' | 'read' | 'control' | 'shell'

export const TEAM_ROLES: readonly TeamRole[] = ['admin', 'member', 'billing']

/// The signed-in person, with their personal team made if this is the first
/// time they've reached the directory.
///
/// Here rather than in `requireAccount`, which every push route calls: those
/// routes never read a team, and shouldn't pay two writes a request for one.
/// Nothing outside the directory can see a team, so making it on first sight
/// of the directory is the same as making it at sign-in.
export async function requireUser(request: Request, env: Env): Promise<string | Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account
  const now = Date.now()
  await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO teams (id, name, personal_of, created_by, created_at) VALUES (?, 'Personal', ?, ?, ?)
       ON CONFLICT (personal_of) DO NOTHING`,
    ).bind(crypto.randomUUID(), account, account, now),
    env.DB.prepare(
      `INSERT INTO memberships (team_id, account_id, role, joined_at)
       SELECT id, ?, 'admin', ? FROM teams WHERE personal_of = ?
       ON CONFLICT (team_id, account_id) DO NOTHING`,
    ).bind(account, now, account),
  ])
  return account
}

export interface Team {
  id: string
  name: string
  personal_of: string | null
}

/// The team and the caller's role in it, or the response to send instead.
///
/// Someone outside a team gets the same 404 as for a team that doesn't exist,
/// so a team id says nothing to a stranger. `allowed` is the roles that may
/// make this call; anyone else in the team gets a 403.
export async function teamFor(
  env: Env,
  account: string,
  teamId: unknown,
  allowed: readonly TeamRole[],
): Promise<{ team: Team; role: TeamRole } | Response> {
  if (typeof teamId !== 'string' || !teamId) return json({ error: 'team' }, 400)
  const row = await env.DB.prepare(
    `SELECT t.id, t.name, t.personal_of, m.role FROM teams t
     JOIN memberships m ON m.team_id = t.id AND m.account_id = ?
     WHERE t.id = ?`,
  )
    .bind(account, teamId)
    .first<Team & { role: TeamRole }>()
  if (!row) return json({ error: 'not found' }, 404)
  if (!allowed.includes(row.role)) return json({ error: 'forbidden' }, 403)
  return { team: { id: row.id, name: row.name, personal_of: row.personal_of }, role: row.role }
}

export interface AuditLine {
  team: string | null
  actor: string
  action: string
  subject?: string | null
  detail?: unknown
}

/// One audit line, as a statement to batch with the change it records, so the
/// change and its line land together or not at all.
///
/// `when` is SQL that must hold for the line to be written, with its own
/// bindings, for a change that is itself conditional (the last-admin guard):
/// the line is written only if the condition that let the change through
/// still holds after it, inside the same transaction.
export function audit(env: Env, line: AuditLine, when?: { sql: string; binds: unknown[] }): D1PreparedStatement {
  const values = [
    line.team,
    line.actor,
    line.action,
    line.subject ?? null,
    line.detail === undefined ? null : JSON.stringify(line.detail),
    Date.now(),
  ]
  if (!when) {
    return env.DB.prepare(
      `INSERT INTO audit (team_id, actor, action, subject, detail, at) VALUES (?, ?, ?, ?, ?, ?)`,
    ).bind(...values)
  }
  return env.DB.prepare(
    `INSERT INTO audit (team_id, actor, action, subject, detail, at) SELECT ?, ?, ?, ?, ?, ? WHERE ${when.sql}`,
  ).bind(...values, ...when.binds)
}

/// The JSON body, or null for one that isn't a JSON object.
export async function bodyOf(request: Request): Promise<Record<string, unknown> | null> {
  try {
    const body = await request.json()
    return body && typeof body === 'object' && !Array.isArray(body) ? (body as Record<string, unknown>) : null
  } catch {
    return null
  }
}

/// A name a person typed: trimmed, 1 to 80 characters, no control characters.
export function nameOf(value: unknown): string | null {
  if (typeof value !== 'string') return null
  const name = value.trim()
  if (!name || name.length > 80 || /[\u0000-\u001f\u007f]/.test(name)) return null
  return name
}

/// Whether a D1 error is a constraint the caller ran into, not a fault.
export function isConstraint(error: unknown): boolean {
  return error instanceof Error && /UNIQUE constraint failed|PRIMARY KEY/.test(error.message)
}
