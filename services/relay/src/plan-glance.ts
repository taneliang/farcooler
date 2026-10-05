/// The plan on the glance (ov-310, ov-268 P8): what the watch, the widgets
/// and the Live Activity say about a board that has a plan.
///
/// The watch has no sockets, so the relay is its only way to hear this. Each
/// runner decides its glance (`crates/daemon/src/plan_glance.rs`) and sends it
/// on a count notice as `plan`: per board with a plan, the board's name, its
/// Needs You count, up to two Now lanes by name and state word, and the lane
/// next up. This file keeps the newest per runner (migration 0020), picks the
/// one board the glance draws (`leadPlan`), and hands it to the card
/// (`withFleet`) and to `/v1/pulse`.
///
/// **Additive.** A runner older than it sends no `plan`; an app older than it
/// ignores the key on the card and in the pulse. Nothing that was sent before
/// changes.
///
/// **Lane names and a board's name, never card text.** The runner sends no
/// more than that, and this file keeps no more than it can bound: anything
/// else on an entry is dropped, and a value of the wrong shape costs its
/// board, never the notice.

import { cut, quietAfterMs, WORKSPACE_BUDGET } from './bounds'

/// A lane in Now: its name, and its state as the runner's store spells it
/// (`building`, `review`, `fixing`, `landing`). The apps say the word.
export interface PlanLane {
  name: string
  state: string
}

/// One board's glance.
export interface PlanBoard {
  workspace: string
  /// The board's Needs You count: its needs-you items plus its themes asking
  /// the owner, the number the Mac says for the board (`WorkspaceNeedsYou`).
  needsYou: number
  now: PlanLane[]
  /// Absent when nothing is queued in the plan.
  next?: string
}

/// How many boards a runner's glance keeps, and Now lanes a board keeps: the
/// runner's own `BOARDS_SENT` and `NOW_SENT`.
export const PLAN_BOARDS_KEPT = 3
export const PLAN_NOW_KEPT = 2

/// The most of a lane's name kept, in bytes. The runner's store bounds a name
/// at 60 characters, and a lane name is a branch-like slug, so this cuts only
/// a name in a wide script.
///
/// Payload arithmetic, beside `ROWS_SHOWN`'s in `index.ts`: the card's `plan`
/// is at most `,"plan":` and a board of a 48-byte workspace, a ten-digit
/// count, two lanes of 60 + 16 bytes and a 60-byte next up, under 360 bytes
/// with its keys. That moves `ROWS_SHOWN`'s fixed part from 774 to about
/// 1,130, and `(4096 - 1130) / 484` is still 6, over the 4 rows a card draws.
/// It is set before the rows, so `STATE_BUDGET` prices it either way.
export const PLAN_NAME_BUDGET = 60

/// A state word: lowercase letters, as the store spells one. A word invented
/// later is kept; the apps say "Unknown" for a word they don't know.
const STATE = /^[a-z]{1,16}$/

/// `raw` as a glance worth keeping: each board that has a name and a count,
/// cut to the bounds above. `null` when it isn't a list at all, which keeps
/// the last one; `[]` is a runner saying none of its boards has a plan.
export function planGlanceOf(raw: unknown): PlanBoard[] | null {
  if (!Array.isArray(raw)) return null
  const boards: PlanBoard[] = []
  for (const entry of raw) {
    if (boards.length === PLAN_BOARDS_KEPT) break
    if (typeof entry !== 'object' || entry === null) continue
    const { workspace, needsYou, now, next } = entry as Record<string, unknown>
    if (typeof workspace !== 'string' || workspace.trim() === '') continue
    if (typeof needsYou !== 'number' || !Number.isInteger(needsYou) || needsYou < 0) continue
    const lanes: PlanLane[] = []
    for (const lane of Array.isArray(now) ? now : []) {
      if (lanes.length === PLAN_NOW_KEPT) break
      const { name, state } = (typeof lane === 'object' && lane !== null ? lane : {}) as Record<string, unknown>
      if (typeof name !== 'string' || name === '' || typeof state !== 'string' || !STATE.test(state)) continue
      lanes.push({ name: cut(name, PLAN_NAME_BUDGET), state })
    }
    boards.push({
      workspace: cut(workspace.trim(), WORKSPACE_BUDGET),
      // `Int` is 32 bits on an arm64_32 watch, so a count past it would fail
      // to decode there and read as none.
      needsYou: Math.min(needsYou, 0x7fffffff),
      now: lanes,
      ...(typeof next === 'string' && next !== '' ? { next: cut(next, PLAN_NAME_BUDGET) } : {}),
    })
  }
  return boards
}

/// The board the glance draws, and how sure the relay is of it: which runner
/// said it, how long ago that runner was last heard (the relay's clock, so
/// no device's clock enters it), and whether it has gone quiet. A quiet
/// runner's plan is still the last word on it, and is drawn with its age.
export interface PlanLead extends PlanBoard {
  runner: string
  heardAgo: number
  quiet?: true
}

/// File what a notice said about this runner's plan, if it said anything,
/// and say whether that moved the board the glance draws.
///
/// Overwritten, never merged: the glance is the runner's reading now, as its
/// count is. Every other token of the same install is this runner under an
/// older pairing, and is cleared, as `needs_you` is. `plan_at` moves only when
/// the glance itself did, so "the plan that moved last" is never a count
/// notice about something else (review H1).
///
/// The answer is what keeps a count notice that moved only another board, or
/// another runner's, from pushing an identical card (review M3).
export async function storePlan(
  db: D1Database,
  daemon: { id: string; account_id: string },
  install: string | null,
  raw: unknown,
): Promise<boolean> {
  const boards = planGlanceOf(raw)
  if (boards === null) return false
  const now = Date.now()
  const before = JSON.stringify(await leadPlan(db, daemon.account_id, now))
  const glance = JSON.stringify(boards)
  await db.prepare(
    `UPDATE daemons SET plan = ?1, plan_at = CASE WHEN plan IS ?1 THEN plan_at ELSE ?2 END WHERE id = ?3`,
  )
    .bind(glance, now, daemon.id)
    .run()
  if (install !== null) {
    await db.prepare(
      `UPDATE daemons SET plan = NULL, plan_at = NULL WHERE account_id = ? AND install_id = ? AND id != ?`,
    )
      .bind(daemon.account_id, install, daemon.id)
      .run()
  }
  return JSON.stringify(await leadPlan(db, daemon.account_id, now)) !== before
}

/// The one board the account's glance draws, or `null` for none.
///
/// Every runner's newest glance is kept until the runner sends another or is
/// unpaired: a plan nobody touched over a weekend is still the plan, and the
/// owner most needs it then (review H1). So nothing here ages out. Instead:
///
/// - a runner unpaired on purpose (`beat_every` NULL, `/v1/heartbeat` with
///   `withdrawn`) is never drawn, and the withdrawal clears its plan (M2);
/// - a runner that went quiet still leads with its last word, marked `quiet`
///   with how long ago it was heard, so a surface says "Can't reach Studio"
///   rather than "No board has a plan" (H2);
/// - a runner still beating outranks a quiet one, a board with something
///   needing the owner outranks one without (M4), and then the plan that
///   moved last.
export async function leadPlan(db: D1Database, account: string, now: number): Promise<PlanLead | null> {
  const held = await db.prepare(
    `SELECT id, label, name, plan, plan_at, last_seen_at, beat_every FROM daemons
     WHERE account_id = ? AND plan IS NOT NULL AND beat_every IS NOT NULL
       AND (expires_at IS NULL OR expires_at > ?)`,
  )
    .bind(account, now)
    .all<{
      id: string
      label: string
      name: string | null
      plan: string
      plan_at: number | null
      last_seen_at: number | null
      beat_every: number
    }>()
  const leads: { lead: PlanLead; at: number; id: string }[] = []
  for (const row of held.results ?? []) {
    let boards: PlanBoard[] | null = null
    try {
      boards = planGlanceOf(JSON.parse(row.plan))
    } catch {
      continue
    }
    if (boards === null || boards.length === 0) continue
    const heardAgo = Math.max(0, now - (row.last_seen_at ?? row.plan_at ?? now))
    const quiet = heardAgo > quietAfterMs(row.beat_every)
    leads.push({
      lead: { ...boards[0], runner: row.name || row.label, heardAgo, ...(quiet ? { quiet: true as const } : {}) },
      at: row.plan_at ?? 0,
      id: row.id,
    })
  }
  leads.sort((a, b) =>
    Number(a.lead.quiet ?? false) - Number(b.lead.quiet ?? false) ||
    Number(b.lead.needsYou > 0) - Number(a.lead.needsYou > 0) ||
    b.at - a.at ||
    (a.id < b.id ? -1 : 1))
  return leads[0]?.lead ?? null
}
