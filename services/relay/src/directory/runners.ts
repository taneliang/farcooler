/// Runners in a team, and who the directory says may reach each one.
///
/// Registering a runner replaces push pairing (`/v1/daemons`): it names the
/// team, requires the runner's install id, and makes the person registering it
/// the runner's owner. The token it returns is an ordinary `daemons` row, so
/// `/v1/notify` and `/v1/heartbeat` take it unchanged.
///
/// Nothing here grants access. Adding someone to a runner needs a grant signed
/// by a runner admin's device (phases 4 and 5), and the only role written here
/// is the owner, who is already the runner's admin. Removing a role is allowed
/// from the directory alone, because a removal can only take access away.

import { installKey, json, sha256, TOKEN_LIFETIME_MS, type Env } from '../index'
import { audit, bodyOf, isConstraint, nameOf, requireUser, teamFor, type RunnerRole } from './common'

/// The key a runner is registered under: global, unlike `installKey`, which is
/// per account. One runner is one row in the whole directory.
export async function runnerKey(install: string): Promise<string> {
  return await sha256(`directory-runner:${install}`)
}

const INSTALL = /^[A-Za-z0-9-]{1,64}$/

/// Add a runner to a team, or rotate the token of one already in it.
///
/// `team` defaults to the caller's personal team. A runner already in the
/// directory can be registered again only by its owner or an admin of it, in
/// the same team, and that rotates its token: the old one stops working at
/// once. Anyone else gets a 409 that names no team. Moving a runner between
/// teams is a transfer, which both teams' admins confirm, and comes later.
export async function registerRunner(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (!body) return json({ error: 'body' }, 400)
  const install = body.install
  if (typeof install !== 'string' || !INSTALL.test(install)) return json({ error: 'install' }, 400)
  const name = body.name === undefined ? 'Runner' : nameOf(body.name)
  if (!name) return json({ error: 'name' }, 400)

  let teamId = body.team
  if (teamId === undefined) {
    const personal = await env.DB.prepare(`SELECT id FROM teams WHERE personal_of = ?`).bind(account).first<{ id: string }>()
    teamId = personal?.id
  }
  const found = await teamFor(env, account, teamId, ['admin', 'member'])
  if (found instanceof Response) return found
  const team = found.team.id

  const key = await runnerKey(install)
  const token = crypto.randomUUID().replaceAll('-', '') + crypto.randomUUID().replaceAll('-', '')
  const daemonId = crypto.randomUUID()
  const now = Date.now()
  const daemon = env.DB.prepare(
    `INSERT INTO daemons (id, account_id, token_hash, label, created_at, expires_at, install_id)
     VALUES (?, ?, ?, ?, ?, ?, ?)`,
  ).bind(daemonId, account, await sha256(token), name, now, now + TOKEN_LIFETIME_MS, await installKey(account, install))

  const existing = await env.DB.prepare(
    `SELECT n.id, n.team_id, n.daemon_id, r.role FROM runners n
     LEFT JOIN runner_roles r ON r.runner_id = n.id AND r.account_id = ?
     WHERE n.install_key = ?`,
  )
    .bind(account, key)
    .first<{ id: string; team_id: string; daemon_id: string | null; role: RunnerRole | null }>()

  if (existing) {
    if (existing.team_id !== team || (existing.role !== 'owner' && existing.role !== 'admin')) {
      return json({ error: 'registered' }, 409)
    }
    await env.DB.batch([
      daemon,
      env.DB.prepare(`UPDATE runners SET daemon_id = ?, name = ? WHERE id = ?`).bind(daemonId, name, existing.id),
      env.DB.prepare(`DELETE FROM daemons WHERE id = ?`).bind(existing.daemon_id),
      audit(env, { team, actor: account, action: 'runner.token_rotated', subject: existing.id }),
    ])
    return json({ runner: existing.id, team, token })
  }

  const runner = crypto.randomUUID()
  try {
    await env.DB.batch([
      daemon,
      env.DB.prepare(
        `INSERT INTO runners (id, team_id, install_key, name, daemon_id, created_at) VALUES (?, ?, ?, ?, ?, ?)`,
      ).bind(runner, team, key, name, daemonId, now),
      env.DB.prepare(
        `INSERT INTO runner_roles (runner_id, account_id, role, granted_by, granted_at) VALUES (?, ?, 'owner', ?, ?)`,
      ).bind(runner, account, account, now),
      audit(env, { team, actor: account, action: 'runner.registered', subject: runner, detail: { name } }),
    ])
  } catch (error) {
    // Registered by someone else between the read and this write.
    if (isConstraint(error)) return json({ error: 'registered' }, 409)
    throw error
  }
  return json({ runner, team, token })
}

/// A team's runners: each one's owner, who may reach it and as what, and when
/// it was last heard from. For admins and members; a billing member sees none.
export async function listRunners(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, ['admin', 'member'])
  if (found instanceof Response) return found

  const [runners, roles] = await env.DB.batch([
    env.DB.prepare(
      `SELECT n.id, n.name, n.created_at, d.last_seen_at, d.version FROM runners n
       LEFT JOIN daemons d ON d.id = n.daemon_id
       WHERE n.team_id = ? ORDER BY n.name, n.created_at`,
    ).bind(found.team.id),
    env.DB.prepare(
      `SELECT r.runner_id, r.account_id, a.email, r.role, r.granted_at FROM runner_roles r
       JOIN runners n ON n.id = r.runner_id JOIN accounts a ON a.id = r.account_id
       WHERE n.team_id = ? ORDER BY r.granted_at`,
    ).bind(found.team.id),
  ])
  type Role = { runner_id: string; account_id: string; email: string | null; role: RunnerRole; granted_at: number }
  const byRunner = new Map<string, Role[]>()
  for (const role of roles.results as Role[]) {
    byRunner.set(role.runner_id, [...(byRunner.get(role.runner_id) ?? []), role])
  }
  type Runner = { id: string; name: string; created_at: number; last_seen_at: number | null; version: string | null }
  return json({
    runners: (runners.results as Runner[]).map(n => {
      const people = byRunner.get(n.id) ?? []
      const owner = people.find(p => p.role === 'owner')
      return {
        id: n.id,
        name: n.name,
        createdAt: n.created_at,
        lastSeenAt: n.last_seen_at,
        version: n.version,
        owner: owner ? owner.account_id : null,
        // No owner and no admin: nobody in the directory can grant access.
        // Someone with a shell on the box claims it (later).
        unclaimed: !people.some(p => p.role === 'owner' || p.role === 'admin'),
        access: people.map(p => ({ user: p.account_id, email: p.email, role: p.role, grantedAt: p.granted_at })),
      }
    }),
  })
}

/// Take someone's role on a runner away, in the directory.
///
/// A team admin may take anyone's; the runner's owner or an admin of it may
/// too. The owner's role can't be taken this way: ownership moves by transfer
/// (later), and leaving the team removes it. Taking the key off the runner
/// itself is phase 6, through a directive.
export async function removeRunnerRole(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (typeof body?.runner !== 'string' || !body.runner) return json({ error: 'runner' }, 400)
  if (typeof body.user !== 'string' || !body.user) return json({ error: 'user' }, 400)
  const user = body.user

  const runner = await env.DB.prepare(`SELECT id, team_id FROM runners WHERE id = ?`)
    .bind(body.runner)
    .first<{ id: string; team_id: string }>()
  if (!runner) return json({ error: 'not found' }, 404)
  const found = await teamFor(env, account, runner.team_id, ['admin', 'member'])
  // Not in the runner's team: the same 404 as no such runner.
  if (found instanceof Response) return found.status === 403 ? found : json({ error: 'not found' }, 404)

  if (found.role !== 'admin') {
    const mine = await env.DB.prepare(`SELECT role FROM runner_roles WHERE runner_id = ? AND account_id = ?`)
      .bind(runner.id, account)
      .first<{ role: RunnerRole }>()
    if (mine?.role !== 'owner' && mine?.role !== 'admin') return json({ error: 'forbidden' }, 403)
  }

  const target = await env.DB.prepare(`SELECT role FROM runner_roles WHERE runner_id = ? AND account_id = ?`)
    .bind(runner.id, user)
    .first<{ role: RunnerRole }>()
  if (!target) return json({ error: 'not found' }, 404)
  if (target.role === 'owner') return json({ error: 'owner' }, 409)

  await env.DB.batch([
    env.DB.prepare(`DELETE FROM runner_roles WHERE runner_id = ? AND account_id = ? AND role != 'owner'`).bind(runner.id, user),
    audit(env, {
      team: runner.team_id,
      actor: account,
      action: 'runner.role_removed',
      subject: user,
      detail: { runner: runner.id, role: target.role },
    }),
  ])
  return json({ ok: true })
}
