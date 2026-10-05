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

import { cut, quietAfterMs, ROW_RETENTION_MS, WORKSPACE_BUDGET } from './bounds'

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
      needsYou: Math.min(needsYou, 0xffffffff),
      now: lanes,
      ...(typeof next === 'string' && next !== '' ? { next: cut(next, PLAN_NAME_BUDGET) } : {}),
    })
  }
  return boards
}

/// File what a notice said about this runner's plan, if it said anything.
///
/// Overwritten, never merged: the glance is the runner's reading now, as its
/// count is. Every other token of the same install is this runner under an
/// older pairing, and is cleared, as `needs_you` is.
export async function storePlan(
  db: D1Database,
  daemon: { id: string; account_id: string },
  install: string | null,
  raw: unknown,
): Promise<void> {
  const boards = planGlanceOf(raw)
  if (boards === null) return
  await db.prepare(`UPDATE daemons SET plan = ?, plan_at = ? WHERE id = ?`)
    .bind(JSON.stringify(boards), Date.now(), daemon.id)
    .run()
  if (install !== null) {
    await db.prepare(
      `UPDATE daemons SET plan = NULL, plan_at = NULL WHERE account_id = ? AND install_id = ? AND id != ?`,
    )
      .bind(daemon.account_id, install, daemon.id)
      .run()
  }
}

/// The one board the account's glance draws, or `null` for none.
///
/// The first board of the runner whose plan moved last: the runner sends its
/// busiest board first, and the newest word is the plan someone is working.
/// A runner that has gone quiet (`quietAfterMs`) is passed over, since its Now
/// is a claim nobody has vouched for since; so is a glance older than
/// `ROW_RETENTION_MS`, the age at which the relay forgets a runner's count.
export async function leadPlan(db: D1Database, account: string, now: number): Promise<PlanBoard | null> {
  const held = await db.prepare(
    `SELECT id, plan, plan_at, last_seen_at, beat_every FROM daemons
     WHERE account_id = ? AND plan IS NOT NULL AND plan_at >= ?
       AND (expires_at IS NULL OR expires_at > ?)
     ORDER BY plan_at DESC, id`,
  )
    .bind(account, now - ROW_RETENTION_MS, now)
    .all<{ id: string; plan: string; plan_at: number; last_seen_at: number | null; beat_every: number | null }>()
  for (const row of held.results ?? []) {
    const quiet = row.beat_every !== null && row.last_seen_at !== null &&
      now - row.last_seen_at > quietAfterMs(row.beat_every)
    if (quiet) continue
    let boards: PlanBoard[] | null = null
    try {
      boards = planGlanceOf(JSON.parse(row.plan))
    } catch {
      continue
    }
    if (boards !== null && boards.length > 0) return boards[0]
  }
  return null
}
