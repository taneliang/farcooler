/// Teams and the people in them.
///
/// Team admins add people (by invite, `invites.ts`), change their roles and
/// remove them. A team always keeps at least one admin, checked inside the
/// write rather than before it, so two admins removing each other at once
/// can't leave a team with none. A personal team has one person, its admin,
/// for good.

import { json, type Env } from '../index'
import { audit, bodyOf, nameOf, requireUser, teamFor, TEAM_ROLES, type TeamRole } from './common'
import { holds, TRUST_RESET_HOLD_MS, type HoldRow } from './hold'

/// Who the caller is: their teams with their role in each, and their devices.
export async function me(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account

  const [who, teams, devices, confirmed] = await env.DB.batch([
    env.DB.prepare(`SELECT email FROM accounts WHERE id = ?`).bind(account),
    env.DB.prepare(
      `SELECT t.id, t.name, t.personal_of, m.role FROM memberships m JOIN teams t ON t.id = m.team_id
       WHERE m.account_id = ? ORDER BY t.personal_of IS NULL, t.name`,
    ).bind(account),
    env.DB.prepare(
      `SELECT id, label, key_a, key_a_fingerprint, key_b, key_b_fingerprint, node_key, state,
              certified_by, created_at, certified_at, revoked_at, trusted_at, trust_reset
       FROM device_keys WHERE account_id = ? ORDER BY created_at`,
    ).bind(account),
    env.DB.prepare(
      `SELECT c.device_id, t.name, c.confirmed_at FROM device_hold_confirmations c
       JOIN device_keys d ON d.id = c.device_id JOIN teams t ON t.id = c.team_id
       WHERE d.account_id = ? ORDER BY c.confirmed_at`,
    ).bind(account),
  ])
  const confirmations = confirmed.results as { device_id: string; name: string; confirmed_at: number }[]
  const email = (who.results[0] as { email: string | null } | undefined)?.email ?? null
  const held = holds(devices.results as unknown as HoldRow[], Date.now()).since
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
      // When it was trusted on first use: its `createdAt` for one trusted as
      // it arrived, later for one already on the account trusted when it
      // signed in again (R-75), which each app's banner announces. Kept once set; null for
      // a device never trusted on first use, and for rows from before it was kept.
      trustedAt: d.trusted_at,
      // Whether that trust was R-75's reset, a new device's included, rather
      // than the account's first device: what the apps announce as one.
      trustReset: d.trust_reset === 1,
      // When teams' admins are sent its key unless one confirms it first
      // (R-76, `hold.ts`): for a device trusted by the reset, or certified by
      // one, until its hold ends; null otherwise. What each app says the
      // device is waiting out.
      heldUntil: held.has(d.id as string) ? held.get(d.id as string)! + TRUST_RESET_HOLD_MS : null,
      // Each team whose admin lifted its hold, and when (fix round 1, M1):
      // what each app's banner announces, since the push can be cut off.
      holdConfirmations: confirmations.filter(c => c.device_id === d.id).map(c => ({ team: c.name, at: c.confirmed_at })),
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
///
/// With `DEVICE_KEYS_ENFORCED` set (ruling R-72; off by default), each member
/// also comes, to the team's admins, with `keys`: the Key A
/// fingerprint of each of their devices that is trusted or certified (ov-549).
/// That is what a runner admin's scan checks a teammate's code against
/// (`crates/client/src/ceremony/teammate.rs`). A pending device isn't listed:
/// nothing vouched for it, and a stolen session can register one. A revoked
/// one isn't either. Nobody else gets the field at all, so its absence says
/// "not told", and an empty list says "none registered".
///
/// A device trusted by R-75's reset, and any it certified, is held out of
/// `keys` for `TRUST_RESET_HOLD_MS` (ruling R-76, `hold.ts`), unless an admin
/// of this team confirmed it in person after the hold began. It comes instead
/// in `held`, as `{key, until}`, beside `keys` and to the same admins, so an
/// admin's scan of it can say it's waiting out a hold and offer the confirm.
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
  // The people an admin removed, for the admins who can let them back in
  // (`readmit` needs an account id, and nothing else lists one).
  const removed =
    found.role === 'admin'
      ? await env.DB.prepare(
          `SELECT r.account_id, a.email, r.removed_at FROM team_removals r
           JOIN accounts a ON a.id = r.account_id
           WHERE r.team_id = ? ORDER BY r.removed_at DESC LIMIT 100`,
        )
          .bind(found.team.id)
          .all<{ account_id: string; email: string | null; removed_at: number }>()
      : null
  // Joined through the membership, so only this team's members' keys can
  // come back, whoever else holds a device. Every device of theirs is read,
  // in any state, so a chain through a voucher that's no longer confirmed
  // is seen as broken rather than missed (fix round 1, H1).
  const enforced = found.role === 'admin' && env.DEVICE_KEYS_ENFORCED === '1'
  const [devices, confirmations] = enforced
    ? await env.DB.batch([
        env.DB.prepare(
          `SELECT d.id, d.account_id, d.key_a_fingerprint, d.state, d.trust_reset, d.trusted_at, d.certified_by
           FROM device_keys d JOIN memberships m ON m.account_id = d.account_id AND m.team_id = ?
           ORDER BY d.created_at`,
        ).bind(found.team.id),
        env.DB.prepare(`SELECT device_id, confirmed_at FROM device_hold_confirmations WHERE team_id = ?`).bind(found.team.id),
      ])
    : [null, null]
  const keys = devices ? (devices.results as (HoldRow & { account_id: string; key_a_fingerprint: string })[]) : null
  const standing = keys ? holds(keys, Date.now()) : { since: new Map<string, number>(), unrooted: new Set<string>() }
  const since = standing.since
  // Only confirmed devices whose chain reaches a trusted one are listed,
  // in either list.
  const listable = (k: HoldRow) => (k.state === 'trusted' || k.state === 'certified') && !standing.unrooted.has(k.id)
  const confirmedAt = new Map(
    ((confirmations?.results ?? []) as { device_id: string; confirmed_at: number }[]).map(c => [c.device_id, c.confirmed_at]),
  )
  // R-76: held is held until the hold ends, or an admin of THIS team
  // confirmed it after the hold began.
  const isHeld = (id: string) => since.has(id) && !((confirmedAt.get(id) ?? -1) >= since.get(id)!)
  const keysOf = (account: string) =>
    keys!.filter(k => k.account_id === account && listable(k) && !isHeld(k.id)).map(k => k.key_a_fingerprint)
  const heldOf = (account: string) =>
    keys!
      .filter(k => k.account_id === account && listable(k) && isHeld(k.id))
      .map(k => ({ key: k.key_a_fingerprint, until: since.get(k.id)! + TRUST_RESET_HOLD_MS }))
  return json({
    team: { id: found.team.id, name: found.team.name, personal: found.team.personal_of !== null },
    members: rows.results.map(m => ({
      user: m.account_id,
      email: m.email,
      role: m.role,
      joinedAt: m.joined_at,
      ...(keys ? { keys: keysOf(m.account_id), held: heldOf(m.account_id) } : {}),
    })),
    ...(removed ? { removed: removed.results.map(r => ({ user: r.account_id, email: r.email, removedAt: r.removed_at })) } : {}),
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
/// In one transaction:
/// - their membership goes, unless they are the last admin;
/// - an admin's removal is recorded, so no invite lets them back in
///   (`acceptInvite`); only `readmit` does. Leaving on your own records
///   nothing;
/// - their roles on the team's runners go;
/// - each runner they owned passes to its earliest-added admin (R-51), or is
///   left ownerless;
/// - and the runner's push channel follows: a token row they held is moved to
///   the new owner, or deleted if there is none, so a removed person's phone
///   stops hearing about the team's runners and their account can't revoke
///   the runner's token.
///
/// Taking their keys off each runner is phase 6, and the audit line lists the
/// runners it will have to reach. That list is read before the write, so phase
/// 6 should recompute it rather than trust it.
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

  const gone = `NOT EXISTS (SELECT 1 FROM memberships WHERE team_id = ? AND account_id = ?)`
  const statements = [
    env.DB.prepare(
      `DELETE FROM memberships WHERE team_id = ?1 AND account_id = ?2
       AND (role != 'admin' OR (SELECT COUNT(*) FROM memberships WHERE team_id = ?1 AND role = 'admin') > 1)`,
    ).bind(team, user),
    env.DB.prepare(
      `DELETE FROM runner_roles WHERE account_id = ?1
       AND runner_id IN (SELECT id FROM runners WHERE team_id = ?2)
       AND NOT EXISTS (SELECT 1 FROM memberships WHERE team_id = ?2 AND account_id = ?1)`,
    ).bind(user, team),
    ...succession(env, team, user, account, access.results.filter(r => r.role === 'owner').map(r => r.runner_id)),
    audit(
      env,
      {
        team,
        actor: account,
        action: user === account ? 'member.left' : 'member.removed',
        subject: user,
        detail: { runners: access.results.map(r => ({ runner: r.runner_id, role: r.role })) },
      },
      { sql: gone, binds: [team, user] },
    ),
  ]
  if (user !== account) {
    statements.push(
      env.DB.prepare(
        `INSERT INTO team_removals (team_id, account_id, removed_by, removed_at)
         SELECT ?1, ?2, ?3, ?4 WHERE NOT EXISTS (SELECT 1 FROM memberships WHERE team_id = ?1 AND account_id = ?2)
         ON CONFLICT (team_id, account_id) DO UPDATE SET removed_by = excluded.removed_by, removed_at = excluded.removed_at`,
      ).bind(team, user, account, Date.now()),
    )
  }
  const [removed] = await env.DB.batch(statements)
  if ((removed.meta?.changes ?? 0) > 0) return json({ ok: true, runners: access.results.length })
  return json({ error: 'last admin' }, 409)
}

/// That `?2` is no longer in team `?1`: every succession step is a no-op
/// unless the removal it follows went through.
const LEFT = `NOT EXISTS (SELECT 1 FROM memberships WHERE team_id = ?1 AND account_id = ?2)`

/// R-51, as statements to run after `leaving`'s roles on `team`'s runners are
/// deleted, each a no-op unless they really left.
///
/// A runner with no owner passes to its earliest-added admin, ties broken by
/// account id so the one-owner index can never see two. Then any token row
/// `leaving` holds for one of the team's runners moves to that runner's owner
/// (its install hash is per account, so it is cleared, and the runner's next
/// beat writes it again), and one with no owner to move to is deleted.
export function succession(env: Env, team: string, leaving: string, actor: string, owned: string[]): D1PreparedStatement[] {
  return [
    env.DB.prepare(
      `UPDATE runner_roles SET role = 'owner' WHERE rowid IN (
         SELECT r.rowid FROM runner_roles r
         WHERE r.role = 'admin' AND r.runner_id IN (SELECT id FROM runners WHERE team_id = ?1)
           AND NOT EXISTS (SELECT 1 FROM runner_roles o WHERE o.runner_id = r.runner_id AND o.role = 'owner')
           AND NOT EXISTS (SELECT 1 FROM runner_roles x WHERE x.runner_id = r.runner_id AND x.role = 'admin'
                           AND (x.granted_at < r.granted_at OR (x.granted_at = r.granted_at AND x.account_id < r.account_id))))
       AND ${LEFT}`,
    ).bind(team, leaving),
    env.DB.prepare(
      `UPDATE daemons SET install_id = NULL, account_id = (
         SELECT o.account_id FROM runners n JOIN runner_roles o ON o.runner_id = n.id AND o.role = 'owner'
         WHERE n.daemon_id = daemons.id)
       WHERE account_id = ?2
         AND id IN (SELECT n.daemon_id FROM runners n JOIN runner_roles o ON o.runner_id = n.id AND o.role = 'owner'
                    WHERE n.team_id = ?1)
         AND ${LEFT}`,
    ).bind(team, leaving),
    env.DB.prepare(
      `DELETE FROM daemons WHERE account_id = ?2
         AND id IN (SELECT daemon_id FROM runners WHERE team_id = ?1)
         AND ${LEFT}`,
    ).bind(team, leaving),
    env.DB.prepare(
      `INSERT INTO audit (team_id, actor, action, subject, detail, at)
       SELECT ?1, ?3, 'runner.owner_changed', o.account_id, json_object('runner', o.runner_id, 'from', ?2), ?4
       FROM runner_roles o JOIN json_each(?5) j ON j.value = o.runner_id
       WHERE o.role = 'owner' AND ${LEFT}`,
    ).bind(team, leaving, actor, Date.now(), JSON.stringify(owned)),
  ]
}

/// Let someone an admin removed back in, as `member` unless `role` says
/// otherwise (never admin; that is a role change afterwards). The only way
/// past a removal: no invite can do it.
export async function readmit(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, ['admin'])
  if (found instanceof Response) return found
  if (typeof body?.user !== 'string' || !body.user) return json({ error: 'user' }, 400)
  const role = body.role ?? 'member'
  if (role !== 'member' && role !== 'billing') return json({ error: 'role' }, 400)
  const team = found.team.id
  const user = body.user

  const was = `EXISTS (SELECT 1 FROM team_removals WHERE team_id = ? AND account_id = ?)`
  const [added] = await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO memberships (team_id, account_id, role, joined_at)
       SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM team_removals WHERE team_id = ?1 AND account_id = ?2)
       ON CONFLICT (team_id, account_id) DO NOTHING`,
    ).bind(team, user, role, Date.now()),
    audit(env, { team, actor: account, action: 'member.readmitted', subject: user, detail: { role } }, { sql: was, binds: [team, user] }),
    env.DB.prepare(`DELETE FROM team_removals WHERE team_id = ? AND account_id = ?`).bind(team, user),
  ])
  if ((added.meta?.changes ?? 0) === 0) return json({ error: 'not found' }, 404)
  return json({ ok: true })
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
