/// The keys on a runner (ov-510, phase 2 of ov-480).
///
/// A runner reports what is in its `authorized_keys`, inside Far Cooler's fence
/// and out, on the heartbeat it already sends: `takeInventory` below. A
/// runner's token is the only way in, and it reaches only the runner row that
/// token belongs to (`runners.daemon_id`): the body names no runner, so one
/// runner can't post, or overwrite, another's. People read it through
/// `runnerKeys`, with the directory's labels worked out against the devices
/// and roles of the moment.
///
/// Read-only for people: nothing here removes a key, tags one, or grants
/// anything. That is phases 4 to 6.

import { json, type Env } from '../index'
import { bodyOf, requireUser, teamFor } from './common'

/// More keys than this on one runner is cut, not refused: a runner with a
/// hundred thousand lines still beats, and what it sends past the bound would
/// only be a larger row.
export const MAX_KEYS = 500
const MAX_COMMENT = 200

/// A fingerprint as `ssh-keygen -lf` prints a SHA-256 one.
const FINGERPRINT = /^SHA256:[A-Za-z0-9+/]{43}$/
const SCOPES = ['read', 'control', 'host_admin', 'shell', 'other'] as const

interface Stored {
  fingerprint: string
  comment: string
  inside: boolean
  scope: string
  firstSeenAt: number
}

/// One entry of a report, if it is one. Anything else in the list is dropped,
/// so a runner that is wrong about one line is not wrong about all of them.
function entryOf(raw: unknown): Omit<Stored, 'firstSeenAt'> | null {
  if (!raw || typeof raw !== 'object') return null
  const e = raw as Record<string, unknown>
  if (typeof e.fingerprint !== 'string' || !FINGERPRINT.test(e.fingerprint)) return null
  if (typeof e.inside !== 'boolean') return null
  const scope = SCOPES.find(s => s === e.scope) ?? 'other'
  const comment = typeof e.comment === 'string' ? [...e.comment.replace(/[\u0000-\u001f\u007f]/g, ' ')].slice(0, MAX_COMMENT).join('') : ''
  return { fingerprint: e.fingerprint, comment, inside: e.inside, scope }
}

const slot = (e: { fingerprint: string; inside: boolean }) => `${e.inside ? 'in' : 'out'}:${e.fingerprint}`

/// Take what a runner's heartbeat says about its keys. Returns `'send'` when
/// the relay wants the list: it holds none, or holds one for another hash.
///
/// `daemonId` is the authenticated token's, never the body's. A token with no
/// runner row (a pairing from before the directory) has nowhere to put keys,
/// and is not asked for any.
export async function takeInventory(env: Env, daemonId: string, report: unknown): Promise<'send' | undefined> {
  if (!report || typeof report !== 'object') return undefined
  const body = report as Record<string, unknown>
  if (typeof body.hash !== 'string' || !/^[0-9a-f]{64}$/.test(body.hash)) return undefined
  const runner = await env.DB.prepare(`SELECT id FROM runners WHERE daemon_id = ?`).bind(daemonId).first<{ id: string }>()
  if (!runner) return undefined

  const held = await env.DB.prepare(`SELECT hash, entries, baseline_at FROM runner_inventory WHERE runner_id = ?`)
    .bind(runner.id)
    .first<{ hash: string; entries: string; baseline_at: number }>()
  const now = Date.now()
  if (!Array.isArray(body.entries)) {
    if (held?.hash === body.hash) {
      await env.DB.prepare(`UPDATE runner_inventory SET reported_at = ? WHERE runner_id = ?`).bind(now, runner.id).run()
      return undefined
    }
    return 'send'
  }

  const seen = new Map<string, number>()
  for (const e of held ? (JSON.parse(held.entries) as Stored[]) : []) seen.set(slot(e), e.firstSeenAt)
  const entries: Stored[] = []
  for (const raw of body.entries.slice(0, MAX_KEYS)) {
    const e = entryOf(raw)
    if (e) entries.push({ ...e, firstSeenAt: seen.get(slot(e)) ?? now })
  }
  const unparsed = typeof body.unparsed === 'number' && Number.isSafeInteger(body.unparsed) ? Math.max(0, body.unparsed) : 0
  // The runner row can be removed between the read above and this write, and
  // the foreign key then refuses the insert: nothing to keep.
  await env.DB.prepare(
    `INSERT INTO runner_inventory (runner_id, hash, unparsed, entries, baseline_at, reported_at)
     SELECT ?1, ?2, ?3, ?4, ?5, ?5 WHERE EXISTS (SELECT 1 FROM runners WHERE id = ?1)
     ON CONFLICT (runner_id) DO UPDATE SET hash = excluded.hash, unparsed = excluded.unparsed,
       entries = excluded.entries, reported_at = excluded.reported_at`,
  )
    .bind(runner.id, body.hash, unparsed, JSON.stringify(entries), held?.baseline_at ?? now)
    .run()
  return undefined
}

export type KeyLabel = 'known' | 'unknown' | 'orphaned' | 'planted'

/// Why a key has its label, as a code the page words. Not prose.
export type KeyReason =
  | 'granted' //             a trusted or certified device of someone who holds a role here
  | 'no_device' //           no profile claims this key
  | 'new_outside_fence' //   no profile claims it, it is outside the fence, and it is newer than the first report
  | 'revoked_device' //      its device was revoked
  | 'uncertified_device' //  its device is not yet certified, so no grant could have admitted it
  | 'not_in_team' //         its owner has left, or was removed from, the team
  | 'no_grant' //            its owner is in the team and holds no role on this runner

interface Claim {
  fingerprint: string
  account_id: string
  state: string
  role: string | null
  member: number
}

/// Label one key against whoever claims it. The four labels are the design's
/// (ov-480 §3):
///
/// - known: a trusted or certified device of a person who is in the team and
///   holds a role on this runner;
/// - orphaned: a device the directory knows, whose owner no longer has any
///   claim here (left the team, holds no role, or the device was revoked);
/// - planted: no device claims it, it sits outside the fence, and it was not
///   in the first report: someone put it there after the runner was enrolled;
/// - unknown: no device claims it, and it was there from the start or is
///   inside the fence.
///
/// A device that is not yet certified is claimed but not vouched for, and no
/// signed grant could have put its key here, so it reads as unknown.
export function labelKey(
  key: Pick<Stored, 'inside' | 'firstSeenAt'>,
  claim: Claim | undefined,
  baselineAt: number,
): { label: KeyLabel; reason: KeyReason; user: string | null } {
  if (!claim) {
    if (!key.inside && key.firstSeenAt > baselineAt) return { label: 'planted', reason: 'new_outside_fence', user: null }
    return { label: 'unknown', reason: 'no_device', user: null }
  }
  const user = claim.account_id
  if (claim.state === 'revoked') return { label: 'orphaned', reason: 'revoked_device', user }
  if (!claim.member) return { label: 'orphaned', reason: 'not_in_team', user }
  if (!claim.role) return { label: 'orphaned', reason: 'no_grant', user }
  if (claim.state === 'pending') return { label: 'unknown', reason: 'uncertified_device', user }
  return { label: 'known', reason: 'granted', user }
}

/// A runner's keys with the directory's label on each: team admins and members
/// of its team. Someone outside the team gets the same 404 as for no such
/// runner. A runner that has not reported yet has no keys to list.
///
/// Body: `runner`.
export async function runnerKeys(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (typeof body?.runner !== 'string' || !body.runner) return json({ error: 'runner' }, 400)
  const runner = await env.DB.prepare(`SELECT id, team_id FROM runners WHERE id = ?`)
    .bind(body.runner)
    .first<{ id: string; team_id: string }>()
  if (!runner) return json({ error: 'not found' }, 404)
  const found = await teamFor(env, account, runner.team_id, ['admin', 'member'])
  if (found instanceof Response) return found.status === 403 ? found : json({ error: 'not found' }, 404)

  const held = await env.DB.prepare(
    `SELECT hash, unparsed, entries, baseline_at, reported_at FROM runner_inventory WHERE runner_id = ?`,
  )
    .bind(runner.id)
    .first<{ hash: string; unparsed: number; entries: string; baseline_at: number; reported_at: number }>()
  if (!held) return json({ runner: runner.id, reportedAt: null, unparsed: 0, keys: [], counts: counts([]) })

  const stored = JSON.parse(held.entries) as Stored[]
  const claims = await env.DB.prepare(
    `SELECT f.fingerprint, d.account_id, d.state,
            (SELECT role FROM runner_roles r WHERE r.runner_id = ?1 AND r.account_id = d.account_id) AS role,
            EXISTS (SELECT 1 FROM memberships m WHERE m.team_id = ?2 AND m.account_id = d.account_id) AS member
     FROM device_fingerprints f JOIN device_keys d ON d.id = f.device_id
     WHERE f.fingerprint IN (SELECT value FROM json_each(?3))`,
  )
    .bind(runner.id, runner.team_id, JSON.stringify(stored.map(e => e.fingerprint)))
    .all<Claim>()
  const byFingerprint = new Map(claims.results.map(c => [c.fingerprint, c]))

  const keys = stored.map(e => ({
    fingerprint: e.fingerprint,
    comment: e.comment,
    inside: e.inside,
    scope: e.scope,
    firstSeenAt: e.firstSeenAt,
    ...labelKey(e, byFingerprint.get(e.fingerprint), held.baseline_at),
  }))
  return json({ runner: runner.id, reportedAt: held.reported_at, unparsed: held.unparsed, keys, counts: counts(keys) })
}

function counts(keys: { label: KeyLabel }[]): Record<KeyLabel, number> {
  const out: Record<KeyLabel, number> = { known: 0, unknown: 0, orphaned: 0, planted: 0 }
  for (const k of keys) out[k.label]++
  return out
}
