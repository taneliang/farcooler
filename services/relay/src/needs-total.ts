/// One `daemons` row, as `readFleet` reads it.
export interface Machine {
  id: string
  label: string
  /// What the runner calls itself, from its beat. See migration 0015.
  name: string | null
  install_id: string | null
  needs_you: number | null
  needs_you_at: number | null
  /// Worktrees to review, as the runner last said. See migration 0019. Absent
  /// reads as NULL: a runner that never sent one.
  reviews?: number | null
  /// The pull requests among `needs_you`. See migration 0028. Absent reads as
  /// NULL: a runner that never sent one.
  pr_reviews?: number | null
  last_seen_at: number | null
  /// How often it promised to beat, in seconds; NULL for a runner too old to
  /// beat, or one unpaired on purpose.
  beat_every: number | null
  expires_at: number | null
}

/// The needs-you total over one count per runner, with a pull request counted
/// once however many runners were asked (ov-590). Its own file to keep
/// `index.ts` inside its size budget.
///
/// Each runner's `needs_you` includes its pull-request review requests, and
/// runners on one gh login see the same ones. The relay cannot tell which are
/// which, so it sums the rest and takes the largest `pr_reviews` among the
/// runners that sent one. A runner that sent none (an older daemon) is summed
/// whole, as before. A `pr_reviews` above its own `needs_you` is a malformed
/// pair and is clamped, so the rest can never go negative.
export function totalOf(counted: Machine[]): number {
  let rest = 0
  let pulls = 0
  for (const machine of counted) {
    const all = machine.needs_you!
    const pr = Math.min(machine.pr_reviews ?? 0, all)
    rest += all - pr
    pulls = Math.max(pulls, pr)
  }
  return rest + pulls
}
