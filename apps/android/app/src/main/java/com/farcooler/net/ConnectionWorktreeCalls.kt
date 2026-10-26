package com.farcooler.net

import com.farcooler.core.refusalWord
import com.farcooler.model.Worktree
import com.farcooler.net.Connection.RemoveOutcome
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

// A worktree's and a terminal's create, remove and write calls on a runner, out of
// Connection.kt for its size budget (ov-818) and unchanged: each is one call and a
// refresh, and needs nothing of the connection's own state.

/**
 * Remove a worktree. **The most destructive thing this app can ask for.**
 *
 * `confirm` must be the worktree's exact name, unless the worktree is
 * clean, in which case it may be empty.
 *
 * ## What the far end refuses, and where
 *
 * Worth having written down here, because the caller has to refuse some of
 * it FIRST — a refusal that lands after somebody has typed a worktree's name
 * into a destructive confirmation is a refusal that came too late to be
 * useful, which is the whole of `07e75e8`.
 *
 * 1. **Scope.** `worktree.remove` is `Scope::HostAdmin`
 *    (`crates/daemon/src/rpc.rs`), and a phone enrolled through the ceremony
 *    holds `control`. That refusal cannot be predicted from this side — a key
 *    added to `authorized_keys` by hand carries no scope line and reads as
 *    host_admin, so two phones with identical settings can get different
 *    answers — so it is REPORTED rather than pre-empted, and the runner's own
 *    words go in a box.
 * 2. **The main checkout**, twice: the `is_main_checkout` flag, and then a
 *    canonicalized path comparison that deliberately does not trust the flag
 *    (`Service::remove_worktree`). Both run before any terminal is stopped and
 *    before any directory is touched. The caller keeps the door shut on
 *    [Worktree.isMainCheckout] anyway, so nobody is walked through a
 *    ceremony that could never have succeeded.
 * 3. **An untrustworthy tmux inventory**, which is refused outright — and
 *    this is the one that arrives in the WORST order. The confirmation gate
 *    is in `rpc.rs` and this check is in `service.rs` underneath it, so on a
 *    dirty worktree the daemon demands the typed name first and only then
 *    says tmux is unreachable. The caller checks [Fleet.runtimeHealthy] before
 *    it offers the button.
 * 4. **A dirty worktree with the wrong name typed**, which is [RemoveOutcome.NeedsTypedName].
 *
 * Past all four it closes every terminal in the worktree — not a reason to
 * refuse, since closing them is part of what was asked for — then runs
 * `git worktree remove --force` and deletes the worktree row. **The branch
 * survives, and so does everything committed on it.**
 */
suspend fun Connection.removeWorktree(worktree: Worktree, confirm: String): RemoveOutcome {
    val data = try {
        core.call(
            "worktree.remove",
            Connection.args("worktree" to worktree.id, "confirm" to confirm),
        )
    } catch (e: Exception) {
        e.rethrowIfCancellation()
        // Refreshed even on a refusal: several of the refusals above are
        // about state this phone is holding a stale copy of, and the fleet
        // it draws behind the failure should be the current one.
        refresh()
        return RemoveOutcome.Refused(e.message, e.refusalWord)
    }
    refresh()
    return if (data["confirmationRequired"]?.jsonPrimitive?.booleanOrNull == true) {
        RemoveOutcome.NeedsTypedName
    } else {
        RemoveOutcome.Removed
    }
}

/**
 * Create a worktree and branch, with a terminal already in it.
 *
 * [name] names the worktree's directory. The wire key is still `task`, which
 * is what it was called when a worktree carried a typed-out task alongside
 * its directory; renaming the key would strand every shipped app for nothing.
 *
 * `terminal` names what runs there; empty means none, which is what a caller
 * about to create its own agent terminal wants. A shell here, because a
 * worktree with nothing running in it is a directory.
 *
 * [adopt] takes an EXISTING branch over instead of creating one, and it is a
 * different operation rather than a flag on the same one: `rpc.rs` routes it
 * to `Service::adopt_branch`, which checks the branch out into a new
 * worktree instead of forking from HEAD.
 *
 * That makes [name] dead on this path. `adopt_branch` ignores it and names
 * the worktree after the branch's last segment, so callers pass the branch
 * in both places rather than sending something the runner will throw away —
 * which is what the CLI and iOS both do. [branch] must already exist, on
 * this repository or on exactly one of its remotes.
 *
 * False for every caller that predates it, and absent from an older client,
 * which `crates/client/src/ffi.rs` reads as false: the behavior
 * `worktree.create` always had.
 *
 * [workspace] claims the new worktree for that workspace; null leaves it
 * to the runner, which claims it for Main.
 */
suspend fun Connection.createWorktree(
    repository: String,
    name: String,
    branch: String,
    terminal: String = "shell",
    adopt: Boolean = false,
    workspace: String? = null,
): String {
    val pairs = mutableListOf<Pair<String, Any>>(
        "repository" to repository,
        "task" to name,
        "branch" to branch,
        "base" to "",
        "terminal" to terminal,
        "adopt" to adopt,
    )
    // Claimed for the workspace whose screen made it (ruling 8). Absent,
    // as from the drawer, is the repository's Main — what it always was.
    // Never for an implicit workspace, whose id is the repository's.
    if (workspace != null) pairs += "workspace" to workspace
    val data = core.call("worktree.create", Connection.args(*pairs.toTypedArray()))
    return data["id"]?.jsonPrimitive?.contentOrNull
        ?: throw com.farcooler.core.CoreException("The runner created a worktree but didn’t name it.")
}

suspend fun Connection.createTerminal(worktree: String, title: String, preset: String): String {
    val data = core.call(
        "terminal.create",
        Connection.args("worktree" to worktree, "title" to title, "preset" to preset),
    )
    return data["id"]?.jsonPrimitive?.contentOrNull
        ?: throw com.farcooler.core.CoreException("The runner started a terminal but didn’t name it.")
}

/**
 * Send exact bytes to a terminal. [hex] must already be lowercase hex —
 * this does no encoding of its own, because the caller that exists needs
 * the sentence and the carriage return sent as two separate calls, not two
 * strings joined into one payload here.
 */
suspend fun Connection.writeRaw(terminal: String, hex: String) {
    core.call("terminal.write", Connection.args("terminal" to terminal, "hex" to hex))
}
