package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.PullRequest
import com.farcooler.model.StackLink
import com.farcooler.model.StackReply
import com.farcooler.model.Trouble
import com.farcooler.model.Workspace
import com.farcooler.model.driftSentence
import com.farcooler.model.prChecksWord
import com.farcooler.model.prReviewWord
import com.farcooler.model.prStateWord
import com.farcooler.net.Connection
import kotlinx.coroutines.launch

// The two things you can do to a worktree from its row that are not opening it.
//
// **Phase 8**, the last of the parity program: the RPCs that were routed in Rust
// and never called from Kotlin. `stack.get` and `pr.refresh` answer "where does
// this branch sit and did CI pass", which is the question you ask from a phone
// and could not; `workspace.remove_worktree` is the one destructive thing in the
// product, and the reason half this file is about refusing rather than doing.
//
// Both hang off `WorkspaceHeader`'s overflow menu in `FleetScreen`, which is
// where iOS puts the same two — a `DropdownMenu` and not a swipe, per the
// standing Android convention `cb13d31` recorded.
//
// **Every runner is its own [Connection], and both of these take one.** That is
// what keeps them right on a fleet of three: a stack is read from the runner
// holding the repository, and a worktree is removed on the runner it lives on,
// structurally rather than by remembering to pass a host id. See
// `net/FleetRepository.kt` — this app connects every runner at once and must
// never grow a surface that assumes one.
//
// **Nothing in this file has drawn a frame.** There is no emulator and no device
// for this phase. The sentences worth being sure about are pure functions in
// `model/Stack.kt` where `StackTest` can read them; everything about layout and
// colour is reasoning, and is marked as such in the report rather than claimed
// here.

/**
 * The bottom-sheet shell both of these use.
 *
 * The same shape as `ReviewSheetFrame` in `ChangesSheets`, deliberately, and a
 * second copy rather than a shared one: that composable is private to a file
 * whose header spends a paragraph on why review sheets live apart from the diff
 * list, and widening it for one caller would either move it somewhere it does
 * not belong or leave it named for a screen it no longer serves. Fifteen lines
 * of padding is the cheaper of the two. If a third appears, promote it.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun WorkspaceSheetFrame(
    title: String,
    onDismiss: () -> Unit,
    action: @Composable () -> Unit = {},
    content: @Composable ColumnScope.() -> Unit,
) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = state) {
        Column(
            Modifier
                .padding(horizontal = 20.dp)
                .padding(bottom = 16.dp)
                .imePadding()
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    title,
                    style = MaterialTheme.typography.headlineSmall,
                    modifier = Modifier.weight(1f),
                )
                action()
            }
            content()
        }
    }
}

// ---- where a branch sits ----

/**
 * A branch's parent chain, and what GitHub last said along it.
 *
 * **Read-only on purpose.** `stack.set_parent` exists and is deliberately not
 * reached from here: setting a parent changes what every diff in the stack is
 * compared against, and this screen exists to answer "what is the state of
 * this" — the question a phone is for. iOS says the same at `StackView`.
 *
 * The refresh in the title bar is the only thing on it that writes anything
 * anywhere, and what it writes is a GitHub read on somebody's rate limit — see
 * `Connection.refreshPullRequests`, which is `Scope::Control` where the read
 * beside it is `Scope::Read`.
 */
@Composable
fun StackSheet(
    connection: Connection,
    repository: String,
    branch: String,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var reply by remember { mutableStateOf<StackReply?>(null) }
    var loading by remember { mutableStateOf(true) }
    var refreshing by remember { mutableStateOf(false) }

    LaunchedEffect(repository, branch) {
        reply = connection.stack(repository, branch)
        loading = false
    }

    WorkspaceSheetFrame(
        title = "Stack",
        onDismiss = onDismiss,
        action = {
            // Asks GitHub again rather than answering from what was last read —
            // the affordance that exists because a cached "passing" is the one
            // reading that misleads. See [PullRequest.stale].
            IconButton(
                enabled = !refreshing && !loading,
                onClick = {
                    refreshing = true
                    scope.launch {
                        // Replaced, not merged: `pr.refresh` answers with the
                        // whole chain. Kept on a null so a refresh that failed
                        // leaves what was already read on screen rather than
                        // emptying it.
                        reply = connection.refreshPullRequests(repository) ?: reply
                        refreshing = false
                    }
                },
            ) {
                if (refreshing) {
                    CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                } else {
                    Icon(
                        Icons.Outlined.Refresh,
                        contentDescription = "Refresh from GitHub",
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
        },
    ) {
        Text(
            branch,
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.MiddleEllipsis,
        )

        val current = reply
        when {
            loading -> Row(verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(8.dp))
                Text(
                    "Reading…",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            // Two silences, said two different ways: nothing came back, and
            // nothing is there. They look identical on screen if they share a
            // sentence, and only one of them is about this branch.
            //
            // "Nothing came back" has two causes and this side cannot tell them
            // apart — a daemon built before `stack.get` refuses the method in
            // `required_scope`, which is shaped like any other failed call, and
            // a link that has just dropped produces the same nothing. Both are
            // named rather than one guessed: `actions::paste_file` writes that
            // rule down at length, after a confident single answer told somebody
            // their runner was out of date when the terminal had simply died.
            current == null -> SheetNoteText(
                "This runner didn’t answer. Its Far Cooler may be too old for this, " +
                    "or the connection may have dropped."
            )

            current.links.isEmpty() -> SheetNoteText("This branch isn’t part of a stack.")

            // `weight(1f, fill = false)` for the reason `ChangesSheets`' header
            // gives about the same problem: inside a bottom sheet the incoming
            // height is bounded, so a weighted child is measured against a
            // definite maximum, and `fill = false` lets a stack of two branches
            // wrap to two instead of standing at full height with a hole under
            // it. A plain `Column` and no lazy list because a stack is two or
            // three links and recycling that is machinery that never runs.
            else -> Column(
                Modifier
                    .weight(1f, fill = false)
                    .verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (current.cycleDetected) {
                    // Reported rather than followed. A parent chain that loops is
                    // walked as far as it was walked, and drawing it as a clean
                    // stack would draw one that does not exist.
                    WarningLine(
                        "These branches list each other as parents. This is what was walked."
                    )
                }
                for ((index, link) in current.links.withIndex()) {
                    if (index > 0) HorizontalDivider()
                    StackLinkRows(link)
                }
            }
        }
    }
}

@Composable
private fun StackLinkRows(link: StackLink) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            link.branch,
            style = MaterialTheme.typography.titleSmall,
            fontFamily = FontFamily.Monospace,
            maxLines = 1,
            overflow = TextOverflow.MiddleEllipsis,
        )
        FactRow("Parent", link.parentBranch.ifBlank { "—" }, monospace = true)
        if (link.parentGuessed) {
            // Red, not amber, and the same red the guessed-BASE warning uses on
            // the review screen. Amber on this phone means an agent is waiting
            // on you and nothing else — `Theme.attentionColor` — and a guessed
            // parent is not somebody waiting, it is a number on this row that
            // may be measured from the wrong place.
            WarningLine("Its parent was guessed, so these counts may be wrong.")
        }
        FactRow("Against parent", link.driftSentence())
        link.pr?.let { PullRequestRows(it) }
    }
}

@Composable
private fun PullRequestRows(pr: PullRequest) {
    val uri = LocalUriHandler.current
    FactRow("Pull request", "#${pr.number} · ${prStateWord(pr.state)}")
    FactRow(
        "Checks",
        prChecksWord(pr.checks),
        // Colour is spent on the reading that is NEWS. A failing check is news;
        // a passing one is the ordinary outcome and says enough in the word
        // itself. This is `processColor`'s rule — "a running process is the
        // ordinary case and now says nothing at all" — and it is why this
        // deliberately does not follow the Mac and iOS, which tint passing green
        // and pending orange. Green here would be the third meaning of green in
        // this app and orange would be the second meaning of the one colour that
        // has exactly one.
        tint = if (pr.checks == "failing") MaterialTheme.colorScheme.error else null,
    )
    prReviewWord(pr.review)?.let { FactRow("Review", it) }
    if (pr.stale) {
        SheetNoteText("Last read from GitHub a while ago. Refresh to be sure.")
    }
    if (pr.url.isNotEmpty()) {
        FilledTonalButton(onClick = { runCatching { uri.openUri(pr.url) } }) {
            Icon(Icons.AutoMirrored.Outlined.OpenInNew, contentDescription = null, Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
            Text("Open on GitHub")
        }
    }
}

/** A label and its value on one line, the shape a list of facts wants. */
@Composable
private fun FactRow(
    label: String,
    value: String,
    monospace: Boolean = false,
    tint: Color? = null,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
        Text(
            label,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.width(12.dp))
        // The VALUE takes the slack, not a spacer between them. A parent branch
        // name is the long half of this pair and it has to be able to truncate;
        // a weighted spacer would give the row's whole width to the gap and
        // squeeze `feat/some-long-branch` down to nothing instead.
        Text(
            value,
            style = MaterialTheme.typography.bodySmall,
            fontFamily = if (monospace) FontFamily.Monospace else FontFamily.Default,
            color = tint ?: MaterialTheme.colorScheme.onSurface,
            textAlign = TextAlign.End,
            overflow = TextOverflow.MiddleEllipsis,
            maxLines = 2,
            modifier = Modifier.weight(1f),
        )
    }
}

/** A sentence in a sheet for a state that is not a list. */
@Composable
private fun SheetNoteText(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/** Something that may be wrong, in the app's one colour for that. */
@Composable
private fun WarningLine(text: String) {
    Row(verticalAlignment = Alignment.Top) {
        Icon(
            Icons.Outlined.Warning,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.error,
            modifier = Modifier.size(14.dp),
        )
        Spacer(Modifier.width(6.dp))
        Text(
            text,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.error,
        )
    }
}

// ---- removing a worktree ----

/**
 * The whole remove-worktree ceremony: ask, then — only if the runner asks for it
 * — have the name typed.
 *
 * **One composable for both phases, because the two are one decision.** Splitting
 * them would put the outcome switch in the caller, and the caller is a row in a
 * list; the single most important property of this flow is that a refusal lands
 * in the phase that raised it and never in the other one's sentences.
 *
 * ## What this refuses before the runner does, and why each
 *
 * `Connection.removeWorktree` writes down the far end's four gates in order.
 * Two of them are checked here first:
 *
 * - **The primary checkout** is not offered at all, by [FleetScreen] leaving the
 *   menu item out on [Workspace.isMainCheckout]. That flag is the one `07e75e8`
 *   found iOS decoding under the CLI's spelling, so it was false for every
 *   workspace and the phone offered for months to remove the one worktree it
 *   cannot. This app reads the FFI's `isMainCheckout` and `FleetDecodeTest`
 *   transcribes that key, which is what makes the guard here worth anything.
 * - **An unreachable tmux**, which is checked in the FIRST dialog. This is the
 *   gate whose ORDER is wrong at the far end and cannot be fixed there: the
 *   confirmation demand lives in `rpc.rs` and the tmux refusal in the
 *   `service.rs` call underneath it, so on a dirty worktree the daemon asks for
 *   the name to be typed and only THEN says it cannot see tmux. Somebody would
 *   type a worktree's name into a destructive confirmation to be told no, which
 *   is exactly the shape `07e75e8` is a fix for.
 *
 * Scope is deliberately NOT pre-empted — see `Connection.removeWorktree`. It
 * cannot be known from here, and a guess would either hide a button that works
 * or promise one that does not.
 *
 * The daemon is still the thing that makes any of this safe. This keeps people
 * out of ceremonies that could never succeed; it is not what protects the
 * directory.
 */
@Composable
fun RemoveWorktreeCeremony(
    connection: Connection,
    workspace: Workspace,
    onFinished: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    // Not `rememberSaveable`, and deliberately — the same rule `FleetScreen`
    // states for its sheets, with more behind it here. Coming back to a phone
    // that was killed in the background and finding a destructive confirmation
    // already open, half-typed, is the app deciding on somebody's behalf that
    // they still want this. The row and its menu are still there.
    var typing by remember { mutableStateOf(false) }
    var working by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<Trouble?>(null) }

    fun ask(confirm: String) {
        working = true
        failure = null
        scope.launch {
            when (val outcome = connection.removeWorktree(workspace, confirm)) {
                is Connection.RemoveOutcome.Removed -> {
                    working = false
                    onFinished()
                }

                is Connection.RemoveOutcome.NeedsTypedName -> {
                    working = false
                    // From the first dialog this is the ordinary answer for a
                    // dirty worktree and opens the second phase. From the second
                    // phase it means the runner disagrees with the name this
                    // phone typed, which it cannot, unless the fleet moved under
                    // us — so it is reported rather than looping.
                    if (typing) {
                        failure = Trouble(
                            "The runner didn’t accept that name. It may have changed " +
                                "since this list was read."
                        )
                    } else {
                        typing = true
                    }
                }

                is Connection.RemoveOutcome.Refused -> {
                    working = false
                    // Far Cooler's sentence and the runner's words, never
                    // joined — the rule `Trouble` exists for. This side genuinely
                    // does not know why: a scope-denied phone, a repository lock
                    // and a git that would not remove the tree all arrive here
                    // looking the same, and the only account of which is the text
                    // that came back.
                    failure = Trouble(
                        "Removing this worktree didn’t finish.",
                        outcome.message,
                    )
                }
            }
        }
    }

    if (typing) {
        TypedNameSheet(
            workspace = workspace,
            working = working,
            failure = failure,
            onRemove = { ask(it) },
            onDismiss = onFinished,
        )
        return
    }

    val name = workspace.task.ifBlank { workspace.branch }
    val tmuxDown = !fleet.runtimeHealthy
    AlertDialog(
        onDismissRequest = { if (!working) onFinished() },
        title = { Text("Remove worktree for $name?") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(removalPrompt(connection.host.displayLabel, tmuxDown))
                failure?.let { SheetFailure(it) }
            }
        },
        confirmButton = {
            TextButton(
                enabled = !working && !tmuxDown,
                colors = ButtonDefaults.textButtonColors(
                    contentColor = MaterialTheme.colorScheme.error,
                ),
                onClick = { ask("") },
            ) { Text("Remove") }
        },
        dismissButton = {
            TextButton(enabled = !working, onClick = onFinished) { Text("Cancel") }
        },
    )
}

/**
 * The second phase: the worktree has uncommitted work, so removing it needs its
 * name typed exactly.
 *
 * A sheet rather than a second dialog, because this one has a text field: a
 * dialog with a keyboard over it is the shape every other typed input in this
 * app avoids, and `imePadding` on a bottom sheet is how the rest of them handle
 * the keyboard.
 *
 * The button is off until the name matches, so the runner's own comparison —
 * `p.typed_confirmation.trim() != ws.name()` in `crates/daemon/src/rpc.rs` — is
 * a backstop rather than the thing doing the work. `trim()` on this side for the
 * same reason it is on that side: a phone keyboard adds a trailing space and
 * refusing over one would be refusing over the keyboard.
 */
@Composable
private fun TypedNameSheet(
    workspace: Workspace,
    working: Boolean,
    failure: Trouble?,
    onRemove: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    // See `typing` above: nothing about this ceremony survives being put down.
    var typed by remember { mutableStateOf("") }
    val matches = removalNameMatches(workspace.task, typed)

    WorkspaceSheetFrame("Remove worktree", onDismiss) {
        Text(
            "This worktree has uncommitted changes. Type its name to remove it.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        OutlinedTextField(
            value = typed,
            onValueChange = { typed = it },
            label = { Text("Name") },
            placeholder = { Text(workspace.task) },
            singleLine = true,
            enabled = !working,
            modifier = Modifier.fillMaxWidth(),
        )
        failure?.let { SheetFailure(it) }
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(enabled = !working, onClick = onDismiss) { Text("Cancel") }
            Spacer(Modifier.weight(1f))
            if (working) {
                CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(12.dp))
            }
            TextButton(
                enabled = matches && !working,
                colors = ButtonDefaults.textButtonColors(
                    contentColor = MaterialTheme.colorScheme.error,
                ),
                onClick = { onRemove(typed.trim()) },
            ) { Text("Remove") }
        }
    }
}

// ---- the words, where a test can read them ----
//
// Pure and out here rather than built inline, for the reason `ChangesSheets`
// gives at the same place: there is no emulator for this phase, so a sentence
// built inside a composable is a sentence nothing can check. It matters more
// here than there — one of these two sentences is the whole account of a
// destructive action, and the other is the only thing on screen explaining a
// button that is switched off.

/**
 * What the first dialog says: either what will happen, or why it can't.
 *
 * [tmuxDown] is `!Fleet.runtimeHealthy` for the runner this worktree is on. The
 * daemon refuses on the same fact — `Service::remove_worktree` bails on an
 * untrustworthy tmux inventory, because `derive_terminal` reports every terminal
 * as Lost when the inventory is unhealthy, so "nothing is running here" is a lie
 * exactly when tmux is unreachable. Said here in the app's own words rather than
 * as the fleet footer's "tmux unavailable", because this sentence has to explain
 * a Remove button that is switched off.
 *
 * The healthy sentence ends on the reassurance deliberately: "deletes the
 * folder" is the half people read, and the branch surviving is the half that
 * decides whether this is frightening. It is also true — `remove_worktree`
 * removes the worktree and the workspace row and never the branch.
 */
internal fun removalPrompt(runner: String, tmuxDown: Boolean): String =
    if (tmuxDown) {
        "Far Cooler can’t reach tmux on $runner right now, so it can’t tell what’s " +
            "still running in this worktree. It won’t remove one until it can."
    } else {
        "Closes every terminal in it and deletes the folder on $runner. The branch " +
            "and everything committed on it stay."
    }

/**
 * Whether what was typed is the worktree's name, by the runner's own rule.
 *
 * `crates/daemon/src/rpc.rs` compares `p.typed_confirmation.trim()` against
 * `ws.name()`, and `wire::workspace` builds the fleet's `task_name` from that
 * same `ws.name()` — so [expected] here really is the string the daemon will
 * compare against, and not a second name that happens to look like it.
 *
 * `trim()` on this side for the same reason it is on that side: a phone keyboard
 * adds a trailing space and refusing over one would be refusing over the
 * keyboard. Empty [expected] never matches — a workspace whose name did not
 * arrive would otherwise be removable by typing nothing at all, which is the one
 * input this gate must never accept.
 */
internal fun removalNameMatches(expected: String, typed: String): Boolean =
    expected.isNotEmpty() && typed.trim() == expected
