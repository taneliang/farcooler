//! The first message of a pane opened for a task (`opening_prompt`). Moved
//! out of `service.rs` (ov-455), which was at its size ceiling.

/// The first message of a pane opened for a task: where the brief is, and
/// nothing else, and how to tell its orchestrator (ov-455).
///
/// The task is the whole brief. The charter is its workspace orchestrator's,
/// and a task agent isn't told where it is (`PaneWorkspace::charter`); the
/// manager skill puts what an agent needs from it on the task instead. So a
/// task that doesn't say where it goes when it's done goes to review, which
/// leaves the owner or the orchestrator to call it done.
///
/// It carries no free text. The task on the board is the brief, and this
/// says to read it, so the board stays the only place the work is described
/// and a revised task is read fresh rather than replayed from a launch
/// argument. It goes through the same first-launch path as any prompt
/// (`launch_command_with_prompt`), so it is sent once: a restart exports the
/// key again and says nothing.
///
/// `cli` is `shim_binary`'s path, `shell_quote`d, as the manager skill gets
/// it: a bare `farcooler` on `PATH` can be another channel's CLI talking to
/// another daemon.
///
/// A key from a repository that missed its prefix (`-1`) would read as a
/// flag in a command, so the commands leave it out and let the pane's
/// `FARCOOLER_TASK` name the task, which it does for every key this is
/// given.
///
/// Public for the CLI's tests, which parse every command in it through the
/// CLI's own clap tree; this crate can't reach that tree.
pub fn opening_prompt(cli: &str, key: &str) -> String {
    let arg = if key.starts_with('-') { String::new() } else { format!(" {key}") };
    format!(
        "You're working {key} on this repository's Far Cooler board. Read the task first: \
         {cli} task show{arg}. The task is your whole brief: work to its acceptance items and \
         within its constraints. Record each decision as you make it with {cli} task note{arg} \
         --kind decision --body \"<what, and why>\". If only the owner can decide something, \
         ask with {cli} task ask{arg} --body \"<the question>\" and stop. When you're done, \
         move the task the way it says, or if it doesn't say, to review with {cli} task \
         set{arg} --status in_review. Then, and whenever you're stuck or need a decision \
         from your orchestrator, tell it in one line: {cli} message orchestrator \"<what \
         happened, and what you need>\". Its replies come into your box tagged \"[from the \
         orchestrator]\"."
    )
}
