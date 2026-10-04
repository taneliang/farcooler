//! The call table is how an app reaches a runner, and nothing in the compiler
//! checks that a method the daemon serves has an arm in it.
//!
//! This is the hole that shipped. `crates/daemon` grew `client.list`,
//! `client.enroll` and `client.revoke`; the protocol gave them a capability and
//! a scope; the daemon dispatched them; and `ffi::dispatch` routed none of the
//! three, so the whole of device enrollment was unreachable from every app with
//! nothing failing anywhere — the same shape as `header.rs`, where a function
//! exported and not declared is invisible to every client.
//!
//! That routing is now checked from the protocol's method table rather than
//! from lists typed here. `route` in `src/ffi/phone_path_tests.rs` is a match
//! on every `Method`, and the tests beside it call each route it declares and
//! each wire method the iOS and Android sources name. These lists were five
//! hand-picked groups, and the agent screen's model and config pickers were in
//! none of them.
//!
//! What stays here is the other half of being reachable: the header an app
//! developer reads says these exist.

/// The methods this crate must route for the enrollment ceremony to end in an
/// enrollment. Their scopes and capabilities are `crates/protocol`'s, and the
/// rules about what may be written are `crates/daemon`'s; what belongs here is
/// only that an app can ask.
const ENROLLMENT: [&str; 3] = ["client.list", "client.enroll", "client.revoke"];

/// The worktree methods, for the same reason. `worktree.reorder` is the one
/// this list was extended for: it is the ONLY way a phone can save an order
/// somebody dragged, and an unrouted arm would leave the drag working on screen
/// and forgotten on the next refresh — no error, nothing in a log, and the
/// runner perfectly capable of storing it the whole time.
const WORKTREES: [&str; 5] = [
    "worktree.create",
    "worktree.hide",
    "worktree.unhide",
    "worktree.reorder",
    "worktree.remove",
];

/// The board's reads. A phone's board with no arm behind it would be a row
/// that opens onto "couldn't read this board" on every runner there is, with
/// the daemon serving the method the whole time.
const BOARD: [&str; 2] = ["task.list", "task.get"];

/// The rollup a phone opens to. Unrouted, the app would fall back to deriving
/// blocked items from the fleet on every runner, and no ask, decision or
/// review would ever reach a phone.
///
/// Answering a decision is `task.note`, and a workspace with no orchestrator
/// is a dead end without `workspace.start_orchestrator` (ruling 8).
const NEEDS_YOU: [&str; 3] = ["needs_you", "task.note", "workspace.start_orchestrator"];

/// Closing a pane: a stop, then a remove, from both phones (`Connection.close`
/// on iOS and Android). `terminal.remove` had no arm, so every close stopped
/// the pane and left it standing, dead, with the error swallowed.
const TERMINALS: [&str; 2] = ["terminal.stop", "terminal.remove"];

/// The agent screen's calls. Both phones send every one of them, and the
/// header listed none (ov-115's review): an app developer reading it could not
/// learn that a pane's prompt, picker or queued message has a method at all.
/// `terminal.agent_subscribe` is the poll behind the live stream.
const AGENT: [&str; 13] = [
    "terminal.set_pane_mode",
    "terminal.agent_subscribe",
    "terminal.agent_prompt",
    "terminal.draft_prompt",
    "terminal.agent_answer",
    "terminal.agent_set_mode",
    "terminal.agent_set_model",
    "terminal.agent_set_config",
    "terminal.agent_edit_queued",
    "terminal.agent_cancel_queued",
    "terminal.agent_steer_queued",
    "terminal.agent_cancel",
    "agent_queue",
];

/// The header says so: an app developer reads that file to find out what may
/// be passed to `farcooler_client_call`.
#[test]
fn the_header_tells_an_app_developer_these_exist() {
    const HEADER: &str = include_str!("../include/farcooler_client.h");
    for method in ENROLLMENT
        .iter()
        .chain(WORKTREES.iter())
        .chain(BOARD.iter())
        .chain(TERMINALS.iter())
        .chain(NEEDS_YOU.iter())
        .chain(AGENT.iter())
        .copied()
    {
        assert!(
            HEADER.contains(method),
            "{method} is routed but undocumented: nobody will find it"
        );
    }
}
