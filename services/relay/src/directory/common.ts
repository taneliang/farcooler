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

/// What no name keeps (ov-560 R1, fix round 1 Low 2):
/// - format characters (Unicode Cf: the bidi controls, zero-width characters
///   and tags), which can reverse or hide the text around a name;
/// - control characters and line and paragraph separators (Cc, Zl, Zp), and
///   the letters that draw as nothing (ㅤ U+3164, ᅟ U+115F, ⠀ U+2800);
/// - opening and closing quotes (Pi, Pf), and the double quotes and
///   lookalikes that are neither (`"`, `` ` ``, „ ″ ‶ ＂ 〝 〞 〟 ❝ ❞ ˮ ״ 〃).
///   Any of them can seem to close the quotes a notice puts around a name and
///   go on with a sentence of its own.
///
/// The apostrophe ’ (U+2019, a Pf) stays only between two letters, once: it's
/// in every default iPhone and Mac name ("Bob’s iPhone"). AgentKit's,
/// Android's and the web's `shownLabel` follow the same rule, all four
/// against one table, `test/fixtures/device-labels.json`.
const UNSHOWN =
  /(?!\u2019)[\p{Cf}\p{Cc}\p{Zl}\p{Zp}\p{Pi}\p{Pf}"`\u02ee\u05f4\u115f\u201e\u2033\u2036\u2800\u3003\u3164\uff02\u301d-\u301f\u275d\u275e]/gu

/// What `nameOf` refuses outright rather than drops: a name a person typed
/// never holds them.
const REFUSED = /[\p{Cc}\p{Zl}\p{Zp}\u115f\u2800\u3164]/u

const LETTER = /^\p{L}$/u

/// `text` without anything in `UNSHOWN`, and each run of apostrophes made one
/// apostrophe between two letters, or nothing.
export function withoutUnshown(text: string): string {
  const plain = text.replace(UNSHOWN, '')
  return plain.replace(/\u2019+/g, (run, at: number) => {
    const before = Array.from(plain.slice(0, at)).pop() ?? ''
    const after = Array.from(plain.slice(at + run.length))[0] ?? ''
    return LETTER.test(before) && LETTER.test(after) ? '\u2019' : ''
  })
}

/// A name a person typed: refused if it holds anything `REFUSED` names; then
/// without the characters `UNSHOWN` names, trimmed, 1 to 80 characters.
export function nameOf(value: unknown): string | null {
  if (typeof value !== 'string' || REFUSED.test(value)) return null
  const name = withoutUnshown(value).trim()
  if (!name || name.length > 80) return null
  return name
}

/// Whether a D1 error is a constraint the caller ran into, not a fault.
export function isConstraint(error: unknown): boolean {
  return error instanceof Error && /UNIQUE constraint failed|PRIMARY KEY/.test(error.message)
}
