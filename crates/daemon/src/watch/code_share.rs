/// What each pane observed working gets out of a worktree's diff growing.
///
/// Pulled out of `record_worktree_trace` so the arithmetic can be argued with
/// directly, because every one of its four answers is a decision:
///
/// - **No previous reading gives nothing.** A worktree already carrying a
///   4,000-line diff when the daemon starts has not just gained 4,000 lines.
/// - **A shrinking diff gives nothing, never a negative.** A commit does not
///   shrink it — the counts are base-to-worktree — but a rebase, a reset or a
///   base change does, and none of those is work being undone in the last three
///   seconds.
/// - **No observed worker gives nothing.** A person editing in their own editor
///   moves the diff with no pane to put it on, and it is dropped rather than
///   shared out among whoever happens to have a terminal in that worktree.
/// - **Several workers split it evenly.** Neither git nor this daemon can say
///   which pane wrote a line, and several terminals routinely share a worktree.
///   An even split keeps the fleet sum exact and makes each row an
///   approximation, which is the direction that loses least; the alternative,
///   giving the whole to each, would multiply the fleet's upper half by however
///   many panes happened to be open.
pub(super) fn code_share(previous: Option<u32>, total: u32, workers: usize) -> u32 {
    if workers == 0 {
        return 0;
    }
    let Some(previous) = previous else { return 0 };
    total.saturating_sub(previous) / workers as u32
}

