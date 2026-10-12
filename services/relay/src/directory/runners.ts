/// Runners in a team, and who the directory says may reach each one.
///
/// Registering a runner replaces push pairing (`/v1/daemons`). It names the
/// team, the runner's install id, and the runner's own key, and it carries
/// proof from the runner itself: an SSH signature by that key over a
/// statement naming the person registering, the team, the install id and the
/// time. The install id alone proves nothing (it is in `ps` and in every
/// worktree, ov-487 review B1), so a runner is identified by its key, which
/// never leaves the box. Whoever can make the runner sign has a shell on it,
/// and the box's shell is the root of trust (ov-480 §2).
///
/// The token it returns is an ordinary `daemons` row, so `/v1/notify` and
/// `/v1/heartbeat` take it unchanged. Nothing here grants access: the only
/// role written is the owner, who is already the runner's admin.

import { installKey, json, sha256, TOKEN_LIFETIME_MS, type Env } from '../index'
import { parsePublicKey, verifySignature, type PublicKey } from '../sshsig'
import { audit, bodyOf, isConstraint, nameOf, requireUser, teamFor, type RunnerRole } from './common'

/// The namespace of a runner's own signatures. Not the grant namespace, so a
/// registration can never be replayed as a grant, or the other way round.
export const RUNNER_NAMESPACE = 'farcooler-runner-v1'

/// How old a runner's statement may be. Long enough for the app to fetch the
/// signature over SSH and post it; short enough that one seen in a log is
/// stale by the time anyone tries it. It also names the account and team, so a
/// replay can only redo what its own signer asked for.
export const STATEMENT_TTL_MS = 10 * 60 * 1000

/// What a runner signs to be registered (`register`) or claimed (`claim`).
/// `relay` is the origin of the relay it is meant for: a proof printed for
/// staging does nothing at production (ov-497 review N3).
export function runnerStatement(
  action: 'register' | 'claim',
  account: string,
  team: string,
  install: string,
  relay: string,
  issuedAt: number,
): string {
  return `farcooler ${action}-runner\naccount ${account}\nteam ${team}\ninstall ${install}\nrelay ${relay}\nissued ${issuedAt}\n`
}

/// The key a runner's install id is stored under: hashed, never the raw id,
/// which is a UUIDv7 and carries the install time.
export async function runnerKey(install: string): Promise<string> {
  return await sha256(`directory-runner:${install}`)
}

const INSTALL = /^[A-Za-z0-9-]{1,64}$/

interface Proven {
  account: string
  team: string
  install: string
  key: PublicKey
  name: string
}

/// Everything registering and claiming share: the session, the team the
/// caller may add a runner to, and the runner's proof. Returns the response
/// to send instead when any of it fails.
async function proven(request: Request, env: Env, action: 'register' | 'claim'): Promise<Proven | Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (!body) return json({ error: 'body' }, 400)
  const install = body.install
  if (typeof install !== 'string' || !INSTALL.test(install)) return json({ error: 'install' }, 400)
  const name = body.name === undefined ? 'Runner' : nameOf(body.name)
  if (!name) return json({ error: 'name' }, 400)
  const key = await parsePublicKey(body.runnerKey)
  if (!key) return json({ error: 'runnerKey' }, 400)
  const issuedAt = body.issuedAt
  if (typeof issuedAt !== 'number' || !Number.isSafeInteger(issuedAt)) return json({ error: 'issuedAt' }, 400)

  let teamId = body.team
  if (teamId === undefined) {
    const personal = await env.DB.prepare(`SELECT id FROM teams WHERE personal_of = ?`).bind(account).first<{ id: string }>()
    teamId = personal?.id
  }
  // R-49: any admin or member may add a runner they hold, and becomes its
  // owner and so its workspace's lead. A billing member may not.
  const found = await teamFor(env, account, teamId, ['admin', 'member'])
  if (found instanceof Response) return found

  const now = Date.now()
  if (issuedAt > now + 60_000 || now - issuedAt > STATEMENT_TTL_MS) return json({ error: 'stale' }, 403)
  const signer = await verifySignature(
    body.proof,
    RUNNER_NAMESPACE,
    runnerStatement(action, account, found.team.id, install, new URL(request.url).origin, issuedAt),
  )
  if (signer?.fingerprint !== key.fingerprint) return json({ error: 'proof' }, 403)
  return { account, team: found.team.id, install, key, name }
}

/// A token row for a runner, under `owner`'s account, so the runner's pushes
/// go to its owner whoever asked for the token.
async function daemonRow(env: Env, id: string, owner: string, token: string, p: Proven): Promise<D1PreparedStatement> {
  const now = Date.now()
  return env.DB.prepare(
    `INSERT INTO daemons (id, account_id, token_hash, label, created_at, expires_at, install_id)
     VALUES (?, ?, ?, ?, ?, ?, ?)`,
  ).bind(id, owner, await sha256(token), p.name, now, now + TOKEN_LIFETIME_MS, await installKey(owner, p.install))
}

function newToken(): string {
  return crypto.randomUUID().replaceAll('-', '') + crypto.randomUUID().replaceAll('-', '')
}

/// Add a runner to a team, or rotate the token of one already in it.
///
/// Body: `team` (default: the caller's personal team), `install`, `name`,
/// `runnerKey`, `issuedAt` and `proof`, the runner's signature over
/// `runnerStatement('register', …)`.
///
/// A runner already in the directory can be registered again only by its
/// owner or an admin of it, in the same team, and that rotates its token. The
/// old token stops working in the same transaction, whichever rotation it
/// races, and the new one stays under the owner's account. Anyone else gets a
/// 409 that names no team, and someone with a shell on the box can
/// `runners/claim` it instead.
export async function registerRunner(request: Request, env: Env): Promise<Response> {
  const p = await proven(request, env, 'register')
  if (p instanceof Response) return p
  const token = newToken()
  const daemonId = crypto.randomUUID()

  const existing = await env.DB.prepare(
    `SELECT n.id, n.team_id, r.role
     FROM runners n LEFT JOIN runner_roles r ON r.runner_id = n.id AND r.account_id = ?
     WHERE n.runner_key_fingerprint = ?`,
  )
    .bind(p.account, p.key.fingerprint)
    .first<{ id: string; team_id: string; role: RunnerRole | null }>()

  if (existing) {
    if (existing.team_id !== p.team || (existing.role !== 'owner' && existing.role !== 'admin')) {
      return json({ error: 'registered' }, 409)
    }
    // Everything is decided inside the write (review R2): the row goes in
    // under whoever owns the runner when the batch commits, and only while the
    // caller still holds owner or admin on it. A rotation racing the owner's
    // removal can't put the token back under him, and one racing the caller's
    // own role removal changes nothing. The install hash is per account and
    // can't be made in SQL, so it starts NULL and the next beat fills it, as
    // succession does.
    const now = Date.now()
    const stillAdmin = `EXISTS (SELECT 1 FROM runner_roles WHERE runner_id = ?2 AND account_id = ?3 AND role IN ('owner', 'admin'))`
    const [inserted] = await env.DB.batch([
      env.DB.prepare(
        `INSERT INTO daemons (id, account_id, token_hash, label, created_at, expires_at, install_id)
         SELECT ?1, o.account_id, ?4, ?5, ?6, ?7, NULL FROM runner_roles o
         WHERE o.runner_id = ?2 AND o.role = 'owner' AND ${stillAdmin}`,
      ).bind(daemonId, existing.id, p.account, await sha256(token), p.name, now, now + TOKEN_LIFETIME_MS),
      // The token the runner points at NOW, not one read before this batch:
      // two rotations at once each delete the other's, and the last one
      // standing is the one the runner row names.
      env.DB.prepare(
        `DELETE FROM daemons WHERE id = (SELECT daemon_id FROM runners WHERE id = ?1)
           AND EXISTS (SELECT 1 FROM daemons WHERE id = ?2)`,
      ).bind(existing.id, daemonId),
      env.DB.prepare(
        `UPDATE runners SET daemon_id = ?1, name = ?2 WHERE id = ?3 AND EXISTS (SELECT 1 FROM daemons WHERE id = ?1)`,
      ).bind(daemonId, p.name, existing.id),
      audit(
        env,
        { team: p.team, actor: p.account, action: 'runner.token_rotated', subject: existing.id },
        { sql: `EXISTS (SELECT 1 FROM daemons WHERE id = ?)`, binds: [daemonId] },
      ),
    ])
    if ((inserted.meta?.changes ?? 0) === 0) return json({ error: 'registered' }, 409)
    return json({ runner: existing.id, team: p.team, token })
  }

  const runner = crypto.randomUUID()
  const now = Date.now()
  try {
    await env.DB.batch([
      await daemonRow(env, daemonId, p.account, token, p),
      env.DB.prepare(
        `INSERT INTO runners (id, team_id, runner_key, runner_key_fingerprint, install_key, name, daemon_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).bind(runner, p.team, p.key.text, p.key.fingerprint, await runnerKey(p.install), p.name, daemonId, now),
      env.DB.prepare(
        `INSERT INTO runner_roles (runner_id, account_id, role, granted_by, granted_at) VALUES (?, ?, 'owner', ?, ?)`,
      ).bind(runner, p.account, p.account, now),
      audit(env, { team: p.team, actor: p.account, action: 'runner.registered', subject: runner, detail: { name: p.name } }),
    ])
  } catch (error) {
    // Registered by someone else between the read and this write.
    if (isConstraint(error)) return json({ error: 'registered' }, 409)
    throw error
  }
  return json({ runner, team: p.team, token })
}

/// Take a runner over with the runner's own signature: `farcooler runner
/// claim`, from a shell on the box.
///
/// For a runner registered to the wrong person or team, or left with nobody
/// to administer it. The box's shell is the root of trust, so a fresh
/// signature over `runnerStatement('claim', …)` is enough: the runner moves to
/// the caller's chosen team with the caller as its owner, every role it had is
/// dropped, and its old token stops working. Both teams' logs say so.
export async function claimRunner(request: Request, env: Env): Promise<Response> {
  const p = await proven(request, env, 'claim')
  if (p instanceof Response) return p
  const existing = await env.DB.prepare(`SELECT id, team_id FROM runners WHERE runner_key_fingerprint = ?`)
    .bind(p.key.fingerprint)
    .first<{ id: string; team_id: string }>()
  if (!existing) return json({ error: 'not found' }, 404)

  const token = newToken()
  const daemonId = crypto.randomUUID()
  const now = Date.now()
  await env.DB.batch([
    await daemonRow(env, daemonId, p.account, token, p),
    env.DB.prepare(`DELETE FROM daemons WHERE id = (SELECT daemon_id FROM runners WHERE id = ?)`).bind(existing.id),
    env.DB.prepare(`DELETE FROM runner_roles WHERE runner_id = ?`).bind(existing.id),
    env.DB.prepare(`UPDATE runners SET team_id = ?, name = ?, daemon_id = ?, install_key = ? WHERE id = ?`).bind(
      p.team,
      p.name,
      daemonId,
      await runnerKey(p.install),
      existing.id,
    ),
    env.DB.prepare(
      `INSERT INTO runner_roles (runner_id, account_id, role, granted_by, granted_at) VALUES (?, ?, 'owner', ?, ?)`,
    ).bind(existing.id, p.account, p.account, now),
    audit(env, { team: existing.team_id, actor: p.account, action: 'runner.claimed_away', subject: existing.id, detail: { to: p.team } }),
    audit(env, { team: p.team, actor: p.account, action: 'runner.claimed', subject: existing.id, detail: { from: existing.team_id, name: p.name } }),
  ])
  return json({ runner: existing.id, team: p.team, token })
}

/// Take a runner out of the directory: a team admin of its team only. Its
/// token stops working and its roles go. The runner itself is untouched and
/// can be registered again, by whoever it signs for.
export async function removeRunner(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (typeof body?.runner !== 'string' || !body.runner) return json({ error: 'runner' }, 400)
  const runner = await env.DB.prepare(`SELECT id, team_id, name FROM runners WHERE id = ?`)
    .bind(body.runner)
    .first<{ id: string; team_id: string; name: string }>()
  if (!runner) return json({ error: 'not found' }, 404)
  const found = await teamFor(env, account, runner.team_id, ['admin', 'member', 'billing'])
  if (found instanceof Response) return json({ error: 'not found' }, 404)
  if (found.role !== 'admin') return json({ error: 'forbidden' }, 403)

  await env.DB.batch([
    env.DB.prepare(`DELETE FROM daemons WHERE id = (SELECT daemon_id FROM runners WHERE id = ?)`).bind(runner.id),
    env.DB.prepare(`DELETE FROM runners WHERE id = ?`).bind(runner.id),
    audit(env, { team: runner.team_id, actor: account, action: 'runner.removed', subject: runner.id, detail: { name: runner.name } }),
  ])
  return json({ ok: true })
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
        // No owner, so no admin either (R-51 promotes one whenever there is
        // one): nobody in the directory can grant access until someone with a
        // shell on the box claims it (`runners/claim`).
        unclaimed: !people.some(p => p.role === 'owner' || p.role === 'admin'),
        access: people.map(p => ({ user: p.account_id, email: p.email, role: p.role, grantedAt: p.granted_at })),
      }
    }),
  })
}

/// Take someone's role on a runner away, in the directory.
///
/// A team admin may take anyone's; the runner's owner or an admin of it may
/// too. The owner's role can't be taken this way: it passes on when the owner
/// leaves the team (R-51), or by `runners/claim`. Taking the key off the runner
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
