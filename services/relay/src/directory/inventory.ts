/// The keys on a runner (ov-510, phase 2 of ov-480).
///
/// A runner reports what sshd would accept from its `authorized_keys` and
/// `authorized_keys2`, inside Far Cooler's fence and out, on the heartbeat it
/// already sends: `takeInventory` below. A runner's token is the only way in,
/// and it reaches only the runner row that token belongs to
/// (`runners.daemon_id`): the body names no runner, so one runner can't post,
/// or overwrite, another's. People read it through `runnerKeys`, with the
/// directory's labels worked out against the devices and roles of the moment.
///
/// **What this is not.** Keys are "as the runner reports them". The runner's
/// token sits in a file the runner's own Unix user can read, and a `control`
/// user is that user (ov-480 §5), so the person this exists to catch can post
/// a false inventory. The labels find mistakes, drift and clumsy plants; they
/// don't prove a runner clean. Nothing here should be worded as if they did.
///
/// Read-only for people: nothing here removes a key, tags one, or grants
/// anything. That is phases 4 to 6.

import { json, type Env } from '../index'
import { audit, bodyOf, requireUser, teamFor } from './common'

/// More keys than this on one runner is cut, not refused: a runner with a
/// hundred thousand lines still beats. The runner sends the ones worth finding
/// first (outside the fence, not ours) and says how many there were in all.
export const MAX_KEYS = 500
const MAX_COMMENT = 200
const MAX_OPTIONS = 200
/// The most key changes one report writes to the audit log. A runner that
/// rewrites its whole file is one line saying so, not five hundred.
const MAX_AUDITED = 25

/// A fingerprint as `ssh-keygen -lf` prints a SHA-256 one.
const FINGERPRINT = /^SHA256:[A-Za-z0-9+/]{43}$/
const SCOPES = ['read', 'control', 'host_admin', 'shell', 'ca', 'other'] as const
const FILES = ['authorized_keys', 'authorized_keys2'] as const
const ERRORS = ['damaged', 'refused', 'too_large', 'unreadable'] as const

/// Characters that make text lie when it is drawn next to a label: C0 and C1
/// controls, DEL, zero-width and directional marks and overrides, the BOM.
const UNSAFE = /[\u0000-\u001f\u007f-\u009f​-‏‪-‮⁠-⁩﻿]/g

interface Stored {
  fingerprint: string
  comment: string
  inside: boolean
  scope: string
  file: string
  options: string
  firstSeenAt: number
}

/// Text a person typed into a file, made safe to show: unsafe characters
/// become spaces, and it is cut to `max` characters.
export function plain(value: unknown, max: number): string {
  return typeof value === 'string' ? [...value.replace(UNSAFE, ' ')].slice(0, max).join('') : ''
}

/// One entry of a report, if it is one. Anything else in the list is dropped,
/// so a runner that is wrong about one line is not wrong about all of them.
function entryOf(raw: unknown): Omit<Stored, 'firstSeenAt'> | null {
  if (!raw || typeof raw !== 'object') return null
  const e = raw as Record<string, unknown>
  if (typeof e.fingerprint !== 'string' || !FINGERPRINT.test(e.fingerprint)) return null
  if (typeof e.inside !== 'boolean') return null
  return {
    fingerprint: e.fingerprint,
    comment: plain(e.comment, MAX_COMMENT),
    inside: e.inside,
    scope: SCOPES.find(s => s === e.scope) ?? 'other',
    file: FILES.find(f => f === e.file) ?? 'authorized_keys',
    options: plain(e.options, MAX_OPTIONS),
  }
}

/// Where a key sits: moving it between the fence and the rest, or between the
/// files, is a new key as far as "since when" goes.
const slot = (e: { fingerprint: string; inside: boolean; file: string }) => `${e.file}:${e.inside ? 'in' : 'out'}:${e.fingerprint}`

interface Held {
  hash: string
  unparsed: number
  baseline_unparsed: number
  total: number
  entries: string
  error: string | null
  baseline_at: number | null
  reported_at: number | null
}

/// Take what a runner's heartbeat says about its keys. Returns `'send'` when
/// the relay wants the list: it holds none, or holds one for another hash.
///
/// `daemonId` is the authenticated token's, never the body's. A token with no
/// runner row (a pairing from before the directory) has nowhere to put keys,
/// and is not asked for any. The caller treats a throw here as "no inventory",
/// never as a failed beat.
export async function takeInventory(env: Env, daemonId: string, report: unknown): Promise<'send' | undefined> {
  if (!report || typeof report !== 'object') return undefined
  const body = report as Record<string, unknown>
  if (typeof body.hash !== 'string' || !/^[0-9a-f]{64}$/.test(body.hash)) return undefined
  const runner = await env.DB.prepare(`SELECT id, team_id FROM runners WHERE daemon_id = ?`)
    .bind(daemonId)
    .first<{ id: string; team_id: string }>()
  if (!runner) return undefined

  const held = await env.DB.prepare(
    `SELECT hash, unparsed, baseline_unparsed, total, entries, error, baseline_at, reported_at FROM runner_inventory WHERE runner_id = ?`,
  )
    .bind(runner.id)
    .first<Held>()
  // Strictly after the baseline, so a key first seen in a later report is
  // always newer than the first report's, however close together they land.
  const now = Math.max(Date.now(), (held?.baseline_at ?? 0) + 1)
  const error = ERRORS.find(e => e === body.error) ?? null
  const actor = `runner:${runner.id}`
  const errorLine = (): D1PreparedStatement[] =>
    (held?.error ?? null) === error
      ? []
      : [audit(env, { team: runner.team_id, actor, action: error ? 'runner.inventory_error' : 'runner.inventory_recovered', subject: runner.id, detail: { error } })]

  if (!Array.isArray(body.entries)) {
    // Nothing could be read: record why, keep the last list and its hash.
    if (error) {
      await env.DB.batch([
        env.DB.prepare(
          `INSERT INTO runner_inventory (runner_id, hash, error, error_at)
           SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM runners WHERE id = ?1)
           ON CONFLICT (runner_id) DO UPDATE SET error = excluded.error, error_at = excluded.error_at`,
        ).bind(runner.id, body.hash, error, now),
        ...errorLine(),
      ])
      return undefined
    }
    if (held?.hash === body.hash && held.reported_at !== null) {
      await env.DB.batch([
        env.DB.prepare(`UPDATE runner_inventory SET reported_at = ?, error = NULL, error_at = NULL WHERE runner_id = ?`).bind(now, runner.id),
        ...errorLine(),
      ])
      return undefined
    }
    return 'send'
  }

  const before = held && held.reported_at !== null ? (JSON.parse(held.entries) as Stored[]) : []
  const seen = new Map<string, number>()
  for (const e of before) seen.set(slot(e), e.firstSeenAt)
  const entries: Stored[] = []
  for (const raw of body.entries.slice(0, MAX_KEYS)) {
    const e = entryOf(raw)
    if (e) entries.push({ ...e, firstSeenAt: seen.get(slot(e)) ?? now })
  }
  const count = (value: unknown, floor: number) =>
    typeof value === 'number' && Number.isSafeInteger(value) && value >= floor && value < 1e9 ? value : floor
  const unparsed = count(body.unparsed, 0)
  const total = count(body.total, entries.length)
  const baselined = held?.baseline_at != null

  // Who put what where, once there is a baseline to compare with, and only
  // when neither list was cut (a cut list can't say what disappeared).
  const lines: D1PreparedStatement[] = []
  if (baselined && held.total === before.length && total === entries.length) {
    const now_ = new Set(entries.map(slot))
    const then = new Set(before.map(slot))
    const changes = [
      ...entries.filter(e => !then.has(slot(e))).map(e => ['runner.key_appeared', e] as const),
      ...before.filter(e => !now_.has(slot(e))).map(e => ['runner.key_disappeared', e] as const),
    ]
    for (const [action, e] of changes.slice(0, MAX_AUDITED)) {
      lines.push(
        audit(env, {
          team: runner.team_id,
          actor,
          action,
          subject: e.fingerprint,
          detail: { runner: runner.id, comment: e.comment, inside: e.inside, file: e.file, scope: e.scope },
        }),
      )
    }
  }

  // The runner row can be removed between the read above and this write, and
  // the foreign key then refuses the insert: nothing to keep.
  await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO runner_inventory (runner_id, hash, unparsed, baseline_unparsed, total, entries, error, error_at, baseline_at, reported_at)
       SELECT ?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8, ?8 WHERE EXISTS (SELECT 1 FROM runners WHERE id = ?1)
       ON CONFLICT (runner_id) DO UPDATE SET hash = excluded.hash, unparsed = excluded.unparsed, total = excluded.total,
         entries = excluded.entries, error = excluded.error, error_at = excluded.error_at, reported_at = excluded.reported_at,
         baseline_unparsed = CASE WHEN runner_inventory.baseline_at IS NULL THEN excluded.baseline_unparsed ELSE runner_inventory.baseline_unparsed END,
         baseline_at = COALESCE(runner_inventory.baseline_at, excluded.baseline_at)`,
    ).bind(runner.id, body.hash, unparsed, total, JSON.stringify(entries), error, error ? now : null, now),
    ...errorLine(),
    ...lines,
  ])
  return undefined
}

export type KeyLabel = 'known' | 'unknown' | 'orphaned' | 'planted'

/// Why a key has its label, as a code the page words. Not prose. On a planted
/// key it is why the key isn't known.
export type KeyReason =
  | 'granted' //             inside the fence, within the role of someone who holds one here
  | 'no_device' //           no profile claims this key
  | 'revoked_device' //      its device was revoked
  | 'uncertified_device' //  its device is not yet certified, so no grant could have admitted it
  | 'not_in_team' //         its owner has left, or was removed from, the team
  | 'no_grant' //            its owner is in the team and holds no role on this runner
  | 'outside_fence' //       its owner holds a role, but the line is not in the fence (or is in authorized_keys2)
  | 'exceeds_grant' //       its owner holds a role, and the line grants more than the role
  | 'unmanaged_line' //      its owner holds a role, but the line inside the fence is not one Far Cooler wrote
  | 'ca_key' //              a cert-authority line: it trusts every certificate its key signs

interface Claim {
  fingerprint: string
  account_id: string
  state: string
  role: string | null
  member: number
}

/// The Far Cooler scopes a role covers. `shell` is Key B's plain line;
/// `host_admin` is an admin's. Owner is an admin with a label.
const COVERS: Record<string, readonly string[]> = {
  owner: ['read', 'control', 'host_admin', 'shell'],
  admin: ['read', 'control', 'host_admin', 'shell'],
  shell: ['read', 'control', 'shell'],
  control: ['read', 'control'],
  read: ['read'],
}

/// Label one key against whoever claims it (ov-480 §3, with the review's
/// tightening, ov-510 H1, H2):
///
/// - known: a trusted or certified device of a person who is in the team and
///   holds a role on this runner, on a line inside the fence that Far Cooler
///   wrote, whose scope the role covers. A bare line above the fence is
///   shell no matter what the role says, so it is never known;
/// - planted: not known, and first seen after the runner's first report,
///   wherever it sits and whoever claims it. A key registered to a throwaway
///   account, or pasted inside the fence, is as new as any other;
/// - orphaned: not planted, and claimed by a device whose owner has no claim
///   here (left the team, no role, revoked) or whose line exceeds the grant;
/// - unknown: the rest: nobody claims it, or it's a CA line, or it's a device
///   not yet certified, and it was there from the start.
export function labelKey(
  key: Pick<Stored, 'inside' | 'firstSeenAt' | 'scope' | 'file'>,
  claim: Claim | undefined,
  baselineAt: number,
): { label: KeyLabel; reason: KeyReason; user: string | null } {
  const user = claim?.account_id ?? null
  let basis: { label: KeyLabel; reason: KeyReason }
  if (key.scope === 'ca') basis = { label: 'unknown', reason: 'ca_key' }
  else if (!claim) basis = { label: 'unknown', reason: 'no_device' }
  else if (claim.state === 'revoked') basis = { label: 'orphaned', reason: 'revoked_device' }
  else if (!claim.member) basis = { label: 'orphaned', reason: 'not_in_team' }
  else if (!claim.role) basis = { label: 'orphaned', reason: 'no_grant' }
  else if (claim.state === 'pending') basis = { label: 'unknown', reason: 'uncertified_device' }
  else if (!key.inside || key.file !== 'authorized_keys') basis = { label: 'orphaned', reason: 'outside_fence' }
  else if (key.scope === 'other') basis = { label: 'orphaned', reason: 'unmanaged_line' }
  else if (!COVERS[claim.role]?.includes(key.scope)) basis = { label: 'orphaned', reason: 'exceeds_grant' }
  else return { label: 'known', reason: 'granted', user }
  if (key.firstSeenAt > baselineAt) return { label: 'planted', reason: basis.reason, user }
  return { ...basis, user }
}

/// A runner's keys with the directory's label on each: team admins and members
/// of its team. Someone outside the team gets the same 404 as for no such
/// runner. A runner that has not reported yet has no keys to list.
///
/// Beside `reportedAt` (when keys were last read) it returns the runner's
/// `lastSeenAt` and `error`, so "beating but not reporting" and "reporting
/// but can't read its file" are visible, not silent. `total` is how many keys
/// the runner read, which may exceed the keys listed; `unparsedSinceBaseline`
/// is how many more unreadable lines there are than at the first report, which
/// is a plant sshd may still obey.
///
/// Keys are as the runner reports them; see the note at the top of this file.
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

  const [heard, held] = await env.DB.batch([
    env.DB.prepare(`SELECT d.last_seen_at FROM runners n LEFT JOIN daemons d ON d.id = n.daemon_id WHERE n.id = ?`).bind(runner.id),
    env.DB.prepare(
      `SELECT unparsed, baseline_unparsed, total, entries, error, error_at, baseline_at, reported_at FROM runner_inventory WHERE runner_id = ?`,
    ).bind(runner.id),
  ])
  const lastSeenAt = (heard.results[0] as { last_seen_at: number | null } | undefined)?.last_seen_at ?? null
  const row = held.results[0] as
    | { unparsed: number; baseline_unparsed: number; total: number; entries: string; error: string | null; error_at: number | null; baseline_at: number | null; reported_at: number | null }
    | undefined
  const empty = { runner: runner.id, reportedAt: null, lastSeenAt, error: row?.error ?? null, errorAt: row?.error_at ?? null, total: 0, unparsed: 0, unparsedSinceBaseline: 0, keys: [], counts: counts([]) }
  if (!row || row.baseline_at === null) return json(empty)

  const stored = JSON.parse(row.entries) as Stored[]
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

  const keys = stored.map(e => {
    const labeled = labelKey(e, byFingerprint.get(e.fingerprint), row.baseline_at!)
    return {
      fingerprint: e.fingerprint,
      comment: e.comment,
      inside: e.inside,
      scope: e.scope,
      file: e.file,
      options: e.options,
      firstSeenAt: e.firstSeenAt,
      ...labeled,
      flagged: labeled.label !== 'known',
    }
  })
  return json({
    ...empty,
    reportedAt: row.reported_at,
    total: row.total,
    unparsed: row.unparsed,
    unparsedSinceBaseline: Math.max(0, row.unparsed - row.baseline_unparsed),
    keys,
    counts: counts(keys),
  })
}

function counts(keys: { label: KeyLabel }[]): Record<KeyLabel, number> {
  const out: Record<KeyLabel, number> = { known: 0, unknown: 0, orphaned: 0, planted: 0 }
  for (const k of keys) out[k.label]++
  return out
}
