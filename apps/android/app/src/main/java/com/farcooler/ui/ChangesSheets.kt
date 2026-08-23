package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.ExpandLess
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.BranchRef
import com.farcooler.model.ChangeCommit
import com.farcooler.model.ChangeSet
import com.farcooler.model.ChangedFile
import com.farcooler.model.ChangesState
import com.farcooler.model.ReviewAgentTarget
import com.farcooler.model.ReviewAnchor
import com.farcooler.model.ReviewComment
import com.farcooler.model.ReviewCommentQueue
import com.farcooler.model.SentReviewBatch
import com.farcooler.model.Trouble
import com.farcooler.net.rethrowIfCancellation
import kotlinx.coroutines.delay
import kotlinx.serialization.json.Json

// The five sheets a review opens, and nothing else.
//
// **Phase 5c**, and the last of phase 5. 5a built the model with nothing to look
// at and 5b built the diff surface, deliberately leaving out every control whose
// destination did not exist yet — no Files button, no History row, no comment
// buttons, no outbox. Those controls and these sheets land together, which is
// the only way either half is honest: a button that opens nothing is worse than
// no button, and a sheet nothing opens is dead code.
//
// ## Why a separate file
//
// `ChangesScreen.kt` is the list, the cards and the bar, and its doc comment
// holds one rule the whole screen depends on — that [ChangesState.rows] IS the
// `LazyColumn`'s item list, so an extra `item {}` anywhere in it moves every
// jump on the screen by one. A sheet emits no items into that list and can
// never break that rule, so keeping the two apart is not filing: it keeps nine
// hundred lines that CANNOT violate the contract out of the file where the
// contract has to be read.
//
// ## What these are, as objects
//
// `ModalBottomSheet`, which is this app's own shape for "pick one of these and
// come straight back" — `RunnerEditorSheet`, `QuickTaskSheet`, `NewWorkspaceSheet`
// and `NewTerminalSheet` are all one. iOS reaches for a `NavigationStack` in a
// sheet with Cancel in the leading slot; the Android equivalent of that shape is
// a bottom sheet, which also puts the content where the thumb already is rather
// than under a navigation bar at the far end of the phone.
//
// **A sheet gets Material's own ground, and that is why accent colour is allowed
// here and forbidden ten feet away.** `ChangesScreen`'s rule — no accent text
// anywhere — is about the diff pane sitting on the TERMINAL's theme-chosen
// background, where how well eleven points of blue can be read depends on which
// palette is in force. `FarCoolerTheme` pins only `surfaceContainerLowest` to
// the terminal's colour; a `ModalBottomSheet` draws on `surfaceContainerLow`,
// which is the platform's own surface under the platform's own scheme. So a
// `TextButton` in here is a `TextButton` on the ground Material chose for it,
// exactly like every other sheet in this app.
//
// ## Heights
//
// Three of these list something whose length is not known in advance — files,
// commits, branches — and use a `LazyColumn` with `weight(1f, fill = false)`.
// That combination is the one that behaves: inside a bottom sheet the incoming
// height is bounded, so the weighted child is measured with a definite maximum
// and a `LazyColumn` composes only what fits, while `fill = false` lets a sheet
// listing three commits wrap to three commits instead of standing at full height
// with a hole under it. The two that are forms scroll a plain `Column` instead,
// because their content is a handful of rows and a lazy list would be paying for
// recycling that never happens.
//
// **Nothing in this file has drawn a frame.** There is no emulator and no device
// for any part of phase 5. Every sentence worth being sure about is a pure
// function at the bottom of this file where `ChangesScreenTest` can read it;
// everything about layout, keyboard and colour is reasoning, and is marked as
// such in the report rather than claimed here.

// ---- which sheet is up ----

/**
 * The sheet the review has open, if any.
 *
 * One value rather than a boolean per sheet, because they are exclusive and
 * five booleans can represent states that cannot happen. iOS keeps three flags
 * and a `ComposeRequest?` and needs the last of those to carry an id, because
 * `sheet(item:)` compares its item and refuses to present a second sheet equal
 * to the first — two notes about the same hunk would be one sheet. **Compose
 * needs no such id**: presentation follows the value being non-null, the
 * composable leaves the composition when it goes null, and its `remember`s go
 * with it, so the second note starts empty because it is a second composition
 * and not because it is a different identity.
 */
internal sealed interface ReviewSheet {
    /** The branch's commits, to pick one out of. */
    data object History : ReviewSheet

    /** Every file in this comparison, to jump to one. */
    data object Index : ReviewSheet

    /** What has been written and not sent. */
    data object Outbox : ReviewSheet

    /** What this worktree is compared against. */
    data object Base : ReviewSheet

    /** Writing one note about one part of the diff. */
    data class Note(val anchor: ReviewAnchor) : ReviewSheet
}

/**
 * The open sheet, across a process death.
 *
 * Saved, and this is the one piece of sheet state that has to be. Four of the
 * five are pickers and losing one costs a tap; the fifth is a composer, and what
 * is in it is a sentence somebody typed that nothing else in the world has a
 * copy of. [ReviewCommentQueue] is careful about exactly that from the moment a
 * note is ADDED, and a note being written is in the window before that — which
 * on this phone is where the process is most likely to be killed, since
 * `docs/jobs-to-be-done.md` F4 is the owner saying a review happens in
 * ninety-second windows with the phone put down in between.
 *
 * So the anchor is written down here and the text is a `rememberSaveable` inside
 * the composer, and coming back from a kill reopens the composer over the note
 * that was being written about the hunk it was about.
 *
 * A string rather than a `Bundle`, because [ReviewAnchor] is already
 * `@Serializable` for the queue's own storage and a second encoding of the same
 * five fields is a second thing to keep in step.
 */
/**
 * Declared ABOVE the saver that reads it, which is not filing.
 *
 * Top-level properties in a file initialize in source order, and 5a already
 * recorded what that costs when it goes the other way: `DiffScope.offered` held
 * a null forever because a `val` list was built while the objects in it were
 * still being loaded, with no warning from the compiler. Nothing here would
 * actually catch fire — the lambdas below read this when they run rather than
 * when they are built — but a reader should not have to work that out.
 */
private val sheetJson = Json { ignoreUnknownKeys = true }

internal val ReviewSheetSaver: Saver<ReviewSheet?, String> = Saver(
    save = { sheet ->
        when (sheet) {
            null -> ""
            ReviewSheet.History -> "history"
            ReviewSheet.Index -> "index"
            ReviewSheet.Outbox -> "outbox"
            ReviewSheet.Base -> "base"
            is ReviewSheet.Note ->
                "note:" + sheetJson.encodeToString(ReviewAnchor.serializer(), sheet.anchor)
        }
    },
    restore = { raw ->
        when {
            raw == "history" -> ReviewSheet.History
            raw == "index" -> ReviewSheet.Index
            raw == "outbox" -> ReviewSheet.Outbox
            raw == "base" -> ReviewSheet.Base
            raw.startsWith("note:") -> runCatching {
                ReviewSheet.Note(
                    sheetJson.decodeFromString(
                        ReviewAnchor.serializer(), raw.removePrefix("note:")
                    )
                )
            }.getOrNull()
            // Deliberately null rather than a default sheet. A record this
            // build cannot read is a sheet nobody asked for, and opening the
            // wrong one over a diff is worse than opening none.
            else -> null
        }
    },
)

// ---- the frame they share ----

/**
 * What every one of these sheets is: a title, a body, and the platform's own
 * dismissal.
 *
 * One frame rather than five, so five sheets cannot come to disagree about their
 * own padding — and because the insets are the part that is easy to get wrong
 * once and then copy four times. `imePadding` and `navigationBarsPadding` are
 * the pair every other sheet in this app already applies, in that order, and the
 * manifest's `adjustResize` is what makes the first of them mean anything.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ReviewSheetFrame(
    title: String,
    onDismiss: () -> Unit,
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
            Text(title, style = MaterialTheme.typography.headlineSmall)
            content()
        }
    }
}

/** The filter every long list in here carries. */
@Composable
private fun FilterField(value: String, label: String, onChange: (String) -> Unit) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
}

/** One sentence in a sheet for a state that is not a list. */
@Composable
private fun SheetNote(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(vertical = 6.dp),
    )
}

/** A section's name, above the rows it names. */
@Composable
private fun SheetSection(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = 8.dp, bottom = 2.dp),
    )
}

/** The tick beside the row that is already what is on screen. */
@Composable
private fun CurrentMark() {
    Icon(
        Icons.Outlined.Check,
        contentDescription = "Showing",
        tint = MaterialTheme.colorScheme.primary,
        modifier = Modifier.size(18.dp),
    )
}

// ---- the file index ----

/**
 * Every file in this comparison, on one screen, without scrolling the diff.
 *
 * The difference between reviewing four files and reviewing forty. A lazy list
 * of forty patches has no table of contents — the only way to learn what a
 * branch touched is to scroll past all of it — and on a phone that is a minute
 * of dragging before the first decision about where to look. The counts are here
 * for the same reason: forty files are not forty equal things, and `+412 -6`
 * beside one name is usually enough to say where to start.
 *
 * **Its whole job is to produce a [com.farcooler.model.Jump], and it computes no
 * index of its own.** Tapping a row calls [onOpen], which is `ChangesStore.expand`
 * — the same call the Next button makes — and the store raises a jump whose key
 * the list resolves against [ChangesState.rows]. That is 5a's contract and the
 * reason this sheet is fifteen lines of behaviour rather than a second copy of
 * the layout: an index that worked out where a file sits by counting would be
 * the layout living in two places, and it would be wrong by one on exactly the
 * branch that regenerated a lockfile.
 *
 * The split is the screen's own — what somebody wrote, then what a tool wrote —
 * because this sheet has to agree with the list behind it about what order the
 * files are in, and with the summary card about which of them are counted apart.
 */
@Composable
internal fun FileIndexSheet(
    state: ChangesState,
    onOpen: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var filter by rememberSaveable { mutableStateOf("") }
    val written = remember(state.files, filter) { matchingFiles(state.handWrittenFiles, filter) }
    val generated = remember(state.files, filter) { matchingFiles(state.generatedFiles, filter) }

    ReviewSheetFrame("Files", onDismiss) {
        FilterField(filter, "Filter files") { filter = it }

        if (written.isEmpty() && generated.isEmpty()) {
            SheetNote(noFilesHere(state.files.isEmpty()))
            return@ReviewSheetFrame
        }

        LazyColumn(Modifier.weight(1f, fill = false)) {
            if (written.isNotEmpty()) {
                item(key = "files.heading") { SheetSection("Files") }
                items(written, key = { "f/" + it.path }) { file ->
                    IndexRow(file, state.isExpanded(file.path)) {
                        onOpen(file.path)
                        onDismiss()
                    }
                }
            }
            if (generated.isNotEmpty()) {
                item(key = "generated.heading") { SheetSection("Generated") }
                items(generated, key = { "g/" + it.path }) { file ->
                    IndexRow(file, state.isExpanded(file.path)) {
                        onOpen(file.path)
                        onDismiss()
                    }
                }
                // Says what the split is FOR, at the one place somebody is
                // looking at both halves at once.
                item(key = "generated.footer") {
                    SheetNote(
                        "Counted apart from the branch’s totals, so a lockfile doesn’t " +
                            "make a branch look bigger than it is."
                    )
                }
            }
        }
    }
}

@Composable
private fun IndexRow(file: ChangedFile, current: Boolean, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            // One phrase, and WITH the directory — which is the opposite of
            // what a heading in the list does, on purpose. See [spokenIndexRow].
            .semantics(mergeDescendants = true) {
                contentDescription = spokenIndexRow(file)
            }
            .heightIn(min = 48.dp)
            .padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            file.status.mark.ifEmpty { "•" },
            style = MaterialTheme.typography.labelSmall,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Bold,
            color = statusColor(file.status),
            modifier = Modifier.width(14.dp),
        )
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(
                file.name,
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
            )
            if (file.directory.isNotEmpty()) {
                Text(
                    file.directory,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.StartEllipsis,
                )
            }
        }
        Spacer(Modifier.width(8.dp))
        FileCounts(file)
        if (current) {
            Spacer(Modifier.width(8.dp))
            CurrentMark()
        }
    }
}

// ---- the history ----

/**
 * The branch, one commit at a time — for the commit somebody has in mind.
 *
 * A picker, and ONLY a picker. Reading a branch commit by commit does not come
 * through here and never should have: a sheet is the right shape for "which one
 * of these" and the wrong shape for "start at the beginning and keep going",
 * which is the reading an agent-authored branch is actually legible in. That
 * reading has controls of its own — `CommitEntry` starts it and the commit
 * header carries Previous and Next — so this is for the commit you already
 * half-remember.
 *
 * The filter matches subject, body, author and sha, because all four are things
 * people half-remember about a commit they are looking for. The body is often
 * where the word actually appears: an agent puts the file it touched in the
 * rationale far more often than in the subject. The sha matches as a PREFIX,
 * because nobody searches for the middle of a hash and a substring match on hex
 * turns every two-character query into noise.
 */
@Composable
internal fun CommitHistorySheet(
    state: ChangesState,
    onWholeBranch: () -> Unit,
    onSelect: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var filter by rememberSaveable { mutableStateOf("") }
    // Read once, when the sheet opens, and handed to every row. A clock read
    // inside a property is an input Compose cannot observe, so the string would
    // freeze until something unrelated forced a redraw — the reason
    // `ChangeCommit.age` takes its `now` as an argument. Nothing here needs to
    // tick: a sheet is open for seconds, and `3h` does not become `4h` inside
    // one.
    val now = remember { System.currentTimeMillis() / 1_000 }
    val shown = remember(state.changeSet.commits, filter) {
        matchingCommits(state.commitsNewestFirst, filter)
    }

    ReviewSheetFrame("History", onDismiss) {
        FilterField(filter, "Filter commits") { filter = it }

        LazyColumn(Modifier.weight(1f, fill = false)) {
            // The way back to everything, at the top, where the thing you undo a
            // choice with belongs. Two more exist in the card this sheet covers,
            // since somebody who has already chosen a commit should not have to
            // open a picker to stop looking at one.
            item(key = "whole.branch") {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable {
                            onWholeBranch()
                            onDismiss()
                        }
                        .heightIn(min = 48.dp)
                        .padding(vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(
                        Icons.Outlined.AccountTree,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(18.dp),
                    )
                    Spacer(Modifier.width(10.dp))
                    Column(Modifier.weight(1f)) {
                        Text("Whole branch", style = MaterialTheme.typography.bodyMedium)
                        Text(
                            wholeBranchDescription(state.changeSet.baseRef),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    if (state.scope.commitSha == null) {
                        Spacer(Modifier.width(8.dp))
                        CurrentMark()
                    }
                }
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            }

            if (state.changeSet.commits.isEmpty()) {
                item(key = "empty") {
                    // The branch itself is empty, which is the ordinary state of
                    // a worktree an agent has only just started in — not a
                    // failure, and not the same as a filter that matched
                    // nothing.
                    SheetNote("This branch hasn’t committed anything yet.")
                }
            } else if (shown.isEmpty()) {
                item(key = "no.match") { SheetNote("No commits match that.") }
            } else {
                item(key = "commits.heading") { SheetSection("Commits") }
            }

            items(shown, key = { it.sha }) { commit ->
                HistoryRow(commit, now, current = state.scope.commitSha == commit.sha) {
                    onSelect(commit.sha)
                    onDismiss()
                }
            }
        }
    }
}

/**
 * Sha, subject, the top of the rationale, author, age and what it changed.
 *
 * The two lines of body are what changes this list from a label into something
 * worth reading. A subject is a label; the body is the closest thing an agent
 * writes to an explanation, and two lines of it is usually the difference
 * between "some commit about retries" and knowing whether this is the one worth
 * opening — which, with ninety seconds, decides the whole window.
 *
 * The counts are the daemon's own, from `--shortstat` on the same `git log` that
 * produced the row, and are ABSENT rather than zero when it could not count
 * them. See [ChangeCommit.counts].
 */
@Composable
private fun HistoryRow(
    commit: ChangeCommit,
    nowSeconds: Long,
    current: Boolean,
    onClick: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .heightIn(min = 48.dp)
            .padding(vertical = 8.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                commit.subject.ifEmpty { "(no subject)" },
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            commit.bodyPreview?.let { preview ->
                Text(
                    preview,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Spacer(Modifier.height(2.dp))
            Text(
                "${commit.short} · ${commit.author} · ${commit.age(nowSeconds)}",
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Spacer(Modifier.width(8.dp))
        // Down the trailing edge rather than along the meta line, which has no
        // room left: sha, author and age already fill a phone's width, and a
        // fourth item on that line truncates the author to an initial.
        Column(horizontalAlignment = Alignment.End) {
            commit.counts?.let { Counts(it.first, it.second) }
            commit.filesChanged?.takeIf { it > 0 }?.let {
                // How WIDE the commit is, which the two line counts do not say:
                // `+300 -40` across one file is a rewrite and across thirty is a
                // rename sweep, and on a branch read one commit at a time that
                // is the difference between opening it now and leaving it for
                // the next window.
                Text(
                    fileCount(it),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        if (current) {
            Spacer(Modifier.width(8.dp))
            CurrentMark()
        }
    }
}

// ---- writing one note ----

/**
 * Writing one note about one part of the diff.
 *
 * The anchor is SHOWN, not implied. What separates a comment from a prompt is
 * that it is about something, and the reader has to be able to see which
 * something before deciding what to say about it — see [ReviewAnchor], which
 * argues the same point from the agent's end.
 *
 * ## The keyboard, which is the whole layout of this sheet
 *
 * The field and the two buttons are ABOVE the block describing the anchor, which
 * is not iOS's order for a reason that is Android's. iOS's `Form` puts Add in
 * the navigation bar, where the keyboard can never cover it; here the button is
 * in the content, so the interactive half is put at the top of the sheet and the
 * reference half below it. Then, whatever the software keyboard does to this
 * window, the field and the button it is answering are in the first two hundred
 * points of the sheet rather than under the keyboard.
 *
 * That is a belt as well as braces. `imePadding` on the frame plus
 * `android:windowSoftInputMode="adjustResize"` in the manifest is what is
 * supposed to lift a sheet clear of the keyboard, and it is exactly what
 * `QuickTaskSheet` already relies on for a text field in a bottom sheet — so
 * this assumes no more than the app already ships. It has not been watched
 * happen, here or there.
 *
 * ## No promise about dictation
 *
 * iOS's footer says to tap the microphone on the keyboard, because on iOS there
 * is one, always, in the same place, and dictating a review note one-handed is
 * the point. **Android has no such guarantee**: the keyboard is a third-party
 * app the user chose, and a microphone key is Gboard's feature rather than the
 * platform's. So this says nothing about dictation. Telling somebody to press a
 * key that may not be on their keyboard is the app describing hardware it cannot
 * see, and the one sentence kept in the footer is the one that changes what
 * happens — that notes are collected and sent together.
 */
@Composable
internal fun CommentComposerSheet(
    anchor: ReviewAnchor,
    onAdd: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    // Saveable, and paired with `ReviewSheetSaver` holding the anchor: between
    // the two, a note half written when Android kills the process is still half
    // written when the app comes back. `ReviewCommentQueue` makes that promise
    // from the moment a note is added; this is the window before that, which on
    // a phone is where the kill actually lands.
    var text by rememberSaveable { mutableStateOf("") }
    val focus = remember { FocusRequester() }

    ReviewSheetFrame("Add a note", onDismiss) {
        OutlinedTextField(
            value = text,
            onValueChange = { text = it },
            label = { Text("What should the agent do about this?") },
            minLines = 3,
            maxLines = 8,
            modifier = Modifier.fillMaxWidth().focusRequester(focus),
        )

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = {
                    onAdd(text)
                    onDismiss()
                },
                enabled = text.isNotBlank(),
            ) {
                Text("Add")
            }
            TextButton(onClick = onDismiss) { Text("Cancel") }
        }

        Text(
            "Notes are collected and sent to the agent together, so it gets one turn " +
                "instead of one per note.",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)

        // `weight(1f, fill = false)` so this takes what the field and the buttons
        // above it left rather than asking for the whole sheet: a quote is the
        // only part of this that can be long, and it is the part that may give
        // way. `fill = false` is what keeps a note about a one-line hunk from
        // standing at full height with a hole under it.
        Column(
            Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            SheetSection("About")
            Text(anchor.file.substringAfterLast('/'), style = MaterialTheme.typography.bodyMedium)
            Text(
                anchor.file,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                capitalizedFirst(anchor.placeDescription),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            anchor.quote?.let { quote ->
                Spacer(Modifier.height(4.dp))
                // Selectable, because a quote is the one thing on this sheet
                // somebody might want to paste back into the note they are
                // writing about it.
                SelectionContainer {
                    Text(
                        quote,
                        style = MaterialTheme.typography.labelSmall,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }

    LaunchedEffect(Unit) {
        // After the presentation animation rather than during it. Focus asked
        // for while a sheet is still sliding up is routinely dropped, and a
        // composer that comes up with no keyboard costs the tap this exists to
        // save. iOS waits 250ms for the same reason and says so.
        delay(220)
        runCatching { focus.requestFocus() }
    }
}

// ---- what has been written, and the two ways out ----

/**
 * Everything written but not yet said, and the controls that say it.
 *
 * **Collect, then send**, which is a decision about the agent rather than about
 * the phone: five notes fired off as five prompts are five turns, each
 * re-reading the files the last one just touched, and the fifth arrives while
 * the agent is still acting on the first. The same five delivered together are
 * one turn against one branch.
 *
 * **Nothing here is ever resent on its own**, and no control in this sheet may
 * imply otherwise. `session/prompt` goes out with `request_no_wait` and its
 * response signals end-of-turn rather than receipt, so a failed send means "this
 * client did not get an answer", which is not the same as "the agent did not get
 * the prompt". So the failure is stated, the notes stay exactly where they were,
 * and the button that would send them a second time says Try again and is
 * pressed by a person. See [ReviewCommentQueue.State.failure].
 *
 * **Put in composer is the second way out, and on this app it is the better
 * one.** It is the Mac's control — see [ReviewCommentQueue.putInComposer] — and
 * 5a recorded, before anything called it, that this app is the Mac's case rather
 * than iOS's: `e23718c` keeps the agent panes of this worktree MOUNTED beside
 * the Changes tab, so a batch dropped into one is a chip away. Text a person can
 * see, edit and send themselves needs no delivery receipt, which is the one
 * thing this whole path cannot offer.
 *
 * A note is removed one at a time, from the row that shows what is being thrown
 * away, and there is no Clear. Everything else on this screen is derived from
 * the daemon and can be discarded because it can be read again; a note is the
 * one thing a person typed, and one button that discards all of them is the
 * wrong thing to put an inch from Send. The delete is a button rather than a
 * swipe, which is this app's own rule — see the parity inventory — and here it
 * has a second reason: a swipe that deletes something unrecoverable wants a
 * confirmation, and a 24-point target that says what it does needs none.
 */
@Composable
internal fun CommentOutboxSheet(
    comments: ReviewCommentQueue,
    agents: List<ReviewAgentTarget>,
    branch: String,
    /**
     * Start a send — `ChangesStore.sendNotes`, which launches on the store's own
     * scope rather than this sheet's.
     *
     * A callback rather than `comments.send` and a `rememberCoroutineScope`,
     * because this sheet can be swiped away while the spinner is up and that
     * would cancel the call mid-flight: the notes would still be pending, no
     * failure would be recorded, and the reader would press Send again. On a
     * path with no delivery receipt that is the duplicate prompt this whole
     * feature refuses to produce on its own, produced by hand instead.
     */
    onSend: (ReviewAgentTarget) -> Unit,
    onPutInComposer: (ReviewAgentTarget, String) -> Unit,
    onDismiss: () -> Unit,
) {
    val state by comments.state.collectAsStateWithLifecycle()
    val now = remember { System.currentTimeMillis() / 1_000 }

    ReviewSheetFrame("Notes", onDismiss) {
        // Above the list rather than in it. A failure is about the whole queue,
        // and one that scrolls out of sight while the reader looks at what it
        // kept is a failure the app has stopped saying.
        state.failure?.let { SheetFailure(it) }

        Column(
            Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()),
        ) {
            if (state.pending.isEmpty()) {
                SheetNote("Nothing written yet. Tap the speech bubble beside a hunk or a file.")
            } else {
                SheetSection("To send")
                for (note in state.pending) {
                    PendingNoteRow(note) { comments.remove(note) }
                }
            }

            if (state.sent.isNotEmpty()) {
                SheetSection("Handed over")
                for (batch in state.sent) {
                    ReceiptRow(batch, now)
                }
                // The receipt, and why it is the only one there can be.
                SheetNote("What went, and when. There’s no delivery receipt to show.")
            }
        }

        if (state.pending.isNotEmpty()) {
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            SendControls(
                state = state,
                agents = agents,
                onSend = onSend,
                onPutInComposer = { target ->
                    val text = comments.putInComposer(target, branch)
                    if (text != null) {
                        onPutInComposer(target, text)
                        onDismiss()
                    }
                },
            )
            Text(
                "Far Cooler can’t tell whether an agent received a prompt, so nothing is " +
                    "ever resent on its own.",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun PendingNoteRow(note: ReviewComment, onRemove: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(vertical = 6.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Column(Modifier.weight(1f)) {
            Text(note.text, style = MaterialTheme.typography.bodyMedium)
            Text(
                notePlace(note.anchor),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.StartEllipsis,
            )
        }
        IconButton(onClick = onRemove) {
            Icon(
                Icons.Outlined.Close,
                contentDescription = "Delete this note",
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(18.dp),
            )
        }
    }
}

/** One batch that left the queue, and the whole text it left with. */
@Composable
private fun ReceiptRow(batch: SentReviewBatch, nowSeconds: Long) {
    var open by remember(batch.id) { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .fillMaxWidth()
                .clickable { open = !open }
                .heightIn(min = 44.dp)
                .padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(noteCount(batch.count), style = MaterialTheme.typography.bodyMedium)
                Text(
                    receiptDetail(batch, nowSeconds),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Icon(
                if (open) Icons.Outlined.ExpandLess else Icons.Outlined.ExpandMore,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(18.dp),
            )
        }
        // In a `DetailBox` rather than a `Text`, because a box is what this app
        // uses to mark a block as a verbatim record rather than as prose.
        if (open) DetailBox(batch.text)
    }
}

/**
 * One button when there is one agent, a menu when there are several, and a
 * sentence when there are none.
 *
 * A picker with one entry is a choice nobody has, and a disabled button with no
 * explanation is the app refusing without saying why: a worktree whose agent has
 * exited has nowhere to send to, and that is a fact about the worktree rather
 * than a fault in the notes.
 */
@Composable
private fun SendControls(
    state: ReviewCommentQueue.State,
    agents: List<ReviewAgentTarget>,
    onSend: (ReviewAgentTarget) -> Unit,
    onPutInComposer: (ReviewAgentTarget) -> Unit,
) {
    if (state.sending) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
            Spacer(Modifier.width(8.dp))
            Text(
                "Sending…",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        return
    }
    if (agents.isEmpty()) {
        SheetNote("No agent is running in this worktree, so there’s nowhere to send these yet.")
        return
    }

    val sendLabel = if (state.failure == null) "Send" else "Try again"
    // Only the panes with a composer on screen. A pane showing its raw terminal
    // is a perfectly good target for a SEND and has no field to put anything in
    // — see `ReviewAgentTarget.showsChat`.
    val composerTargets = agents.filter { it.showsChat }

    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        AgentAction(
            label = sendLabel,
            oneLabel = { "$sendLabel to ${it.name}" },
            icon = Icons.AutoMirrored.Filled.Send,
            targets = agents,
            prominent = true,
            onChoose = onSend,
        )
        if (composerTargets.isNotEmpty()) {
            AgentAction(
                label = "Put in composer",
                oneLabel = { "Put in ${it.name}" },
                icon = Icons.Outlined.Edit,
                targets = composerTargets,
                prominent = false,
                onChoose = onPutInComposer,
            )
        }
    }
}

/** A button when there is one target, and the same button over a menu when several. */
@Composable
private fun AgentAction(
    label: String,
    oneLabel: (ReviewAgentTarget) -> String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    targets: List<ReviewAgentTarget>,
    prominent: Boolean,
    onChoose: (ReviewAgentTarget) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    val only = targets.singleOrNull()
    val text = if (only != null) oneLabel(only) else label

    Box {
        val onClick: () -> Unit = {
            if (only != null) onChoose(only) else open = true
        }
        val body: @Composable () -> Unit = {
            Icon(icon, contentDescription = null, modifier = Modifier.size(16.dp))
            Spacer(Modifier.width(6.dp))
            Text(text, maxLines = 1)
        }
        if (prominent) {
            Button(onClick = onClick) { body() }
        } else {
            FilledTonalButton(onClick = onClick) { body() }
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for (target in targets) {
                DropdownMenuItem(
                    text = { Text(target.name) },
                    onClick = {
                        open = false
                        onChoose(target)
                    },
                )
            }
        }
    }
}

// ---- what this is compared against ----

/**
 * What this worktree's diff is measured from, and how to pin it.
 *
 * The affordance the guessed-base warning has been naming since 5b without being
 * able to open anything. It matters more than its size suggests: a GUESSED base
 * is the only one that can silently produce a wrong diff that looks exactly like
 * a right one, and until this existed a phone could see the warning and do
 * nothing about it.
 *
 * **A list rather than a text field**, and that is the whole design. The daemon
 * validates a ref with `rev-parse --verify` before recording it — see
 * `review_ops::set_base` — so a typo fails cleanly rather than silently, but a
 * phone keyboard is still the worst way in the world to enter `origin/main`
 * between two sets. `branch.list` already knows every ref this repository has,
 * so the list is the input. Remote-tracking refs are in it and marked, because
 * `main` and `origin/main` are different answers and a list of thirty names is
 * exactly where they would otherwise be told apart by nothing.
 *
 * It needs the REPOSITORY rather than the worktree, which is the daemon's own
 * shape. `Workspace.repository` is nullable, because a fleet from an older
 * runner never carried it — so this says so rather than showing an empty list,
 * which would be a claim that the repository has no branches.
 */
@Composable
internal fun BaseBranchSheet(
    set: ChangeSet,
    repositoryId: String?,
    loadBranches: suspend (String) -> List<BranchRef>,
    onChoose: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var filter by rememberSaveable { mutableStateOf("") }
    var branches by remember { mutableStateOf<List<BranchRef>?>(null) }
    var failure by remember { mutableStateOf<Trouble?>(null) }

    LaunchedEffect(repositoryId) {
        val repository = repositoryId ?: return@LaunchedEffect
        try {
            branches = loadBranches(repository)
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            failure = Trouble("Couldn’t read this project’s branches.", e.message)
        }
    }

    val shown = remember(branches, filter) { matchingBranches(branches.orEmpty(), filter) }

    ReviewSheetFrame("Base branch", onDismiss) {
        Text(
            baseDescription(set),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        when {
            repositoryId == null -> SheetNote(
                "This runner didn’t say which project this worktree belongs to, so its " +
                    "branches can’t be listed. A newer Far Cooler on that runner will."
            )

            failure != null -> SheetFailure(failure!!)

            branches == null -> Row(verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(8.dp))
                Text(
                    "Reading branches…",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            else -> {
                FilterField(filter, "Filter branches") { filter = it }
                if (shown.isEmpty()) {
                    SheetNote(noBranchesHere(branches.orEmpty().isEmpty()))
                } else {
                    LazyColumn(Modifier.weight(1f, fill = false)) {
                        items(shown, key = { it.name }) { branch ->
                            BranchRow(branch, current = branch.name == set.baseRef) {
                                onChoose(branch.name)
                                onDismiss()
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun BranchRow(branch: BranchRef, current: Boolean, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .heightIn(min = 48.dp)
            .padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                branch.name,
                style = MaterialTheme.typography.bodyMedium,
                fontFamily = FontFamily.Monospace,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
            )
            Text(
                branchAside(branch),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (current) {
            Spacer(Modifier.width(8.dp))
            CurrentMark()
        }
    }
}

// ---- the words, where a test can read them ----
//
// Pure and out here rather than built inline, for the reason `ChangesScreen`
// gives at the same place: there is no emulator for this phase, so a sentence
// built inside a composable is a sentence nothing can check.

/**
 * The whole path, not the leaf.
 *
 * "daemon" is how somebody asks for everything under `crates/daemon/`, and
 * matching only the filename would answer that with nothing.
 */
internal fun matchingFiles(files: List<ChangedFile>, filter: String): List<ChangedFile> {
    val query = filter.trim().lowercase()
    if (query.isEmpty()) return files
    return files.filter { it.path.lowercase().contains(query) }
}

/** Subject, body, author, and the sha as a PREFIX. See [CommitHistorySheet]. */
internal fun matchingCommits(commits: List<ChangeCommit>, filter: String): List<ChangeCommit> {
    val query = filter.trim().lowercase()
    if (query.isEmpty()) return commits
    return commits.filter {
        it.subject.lowercase().contains(query) ||
            it.bodyText?.lowercase()?.contains(query) == true ||
            it.author.lowercase().contains(query) ||
            it.sha.lowercase().startsWith(query)
    }
}

internal fun matchingBranches(branches: List<BranchRef>, filter: String): List<BranchRef> {
    val query = filter.trim().lowercase()
    if (query.isEmpty()) return branches
    return branches.filter { it.name.lowercase().contains(query) }
}

/**
 * An empty file index is two different facts, and only one of them is about the
 * filter.
 */
internal fun noFilesHere(comparisonIsEmpty: Boolean): String =
    if (comparisonIsEmpty) "Nothing changed in this comparison." else "No files match that."

internal fun noBranchesHere(repositoryIsEmpty: Boolean): String =
    if (repositoryIsEmpty) "This project has no branches yet." else "No branches match that."

/**
 * The base is named when it is known. It is empty until the first read lands,
 * and "Every commit since , at once" is how an unloaded pane would read it out.
 */
internal fun wholeBranchDescription(baseRef: String): String =
    if (baseRef.isEmpty()) "Every commit on this branch, at once"
    else "Every commit since $baseRef, at once"

/**
 * Where the current base came from, said rather than spelled.
 *
 * Every arm of `base_source_name` in `crates/client/src/changes_json.rs` gets a
 * sentence, because the whole point of this sheet is that a reader can tell a
 * RECORDED base from a guessed one — and a screen that only ever said "compared
 * against main" would make the two look identical, which is the exact confusion
 * `ChangeSet.baseIsGuessed` exists to break.
 */
internal fun baseDescription(set: ChangeSet): String {
    val ref = set.baseRef
    if (ref.isEmpty()) return "Nothing is recorded as this worktree’s base yet."
    return when (set.baseSource) {
        "recorded" -> "Pinned to $ref."
        "guessed" -> "Guessed as $ref, so the diff may be wrong. Pin the right one."
        "upstream" -> "$ref, from this branch’s upstream."
        "pr_base" -> "$ref, from the pull request this branch targets."
        "default_branch" -> "$ref, the project’s default branch."
        else -> "Compared against $ref."
    }
}

/** What a branch row says under the name. */
internal fun branchAside(branch: BranchRef): String {
    val parts = mutableListOf(branch.whereItLives)
    if (branch.checkedOut) parts.add("checked out")
    branch.subject.takeIf { it.isNotEmpty() }?.let { parts.add(it) }
    return parts.joinToString(" · ")
}

/**
 * A row of the index, said as one phrase — and WITH its directory.
 *
 * The opposite of `spokenHeading`, which leaves the directory out, and 5b
 * predicted this split before there was a sheet to put it in. A heading is
 * PASSED on the way down a list of forty files, so `crates/daemon/src` read
 * aloud once per file is most of what a screen reader would say. A row of this
 * sheet is CHOSEN, and two files called `mod.rs` are two identical rows without
 * it — which is the one thing an index may not be.
 *
 * The name first and the directory as an aside, rather than the raw path: a path
 * read letter by letter puts the part that identifies the file last, behind the
 * `crates/` that every row on the branch begins with.
 */
internal fun spokenIndexRow(file: ChangedFile): String {
    val heading = spokenHeading(file)
    if (file.directory.isEmpty()) return heading
    val name = file.name
    return heading.replaceFirst(name, "$name, in ${file.directory}")
}

/** Where a pending note is about, for the outbox row. */
internal fun notePlace(anchor: ReviewAnchor): String {
    val leaf = anchor.file.substringAfterLast('/')
    if (anchor.firstLine == null) return leaf
    return "$leaf · ${anchor.placeDescription}"
}

internal fun noteCount(count: Int): String = if (count == 1) "1 note" else "$count notes"

/** The outbox row above the review bar. */
internal fun notesWaiting(count: Int): String =
    if (count == 1) "1 note for the agent" else "$count notes for the agent"

internal fun fileCount(count: Int): String = if (count == 1) "1 file" else "$count files"

/**
 * What happened to a batch, and when.
 *
 * The two verbs are different promises and must not read the same. A SEND was
 * handed to an agent with no way to confirm it arrived; a batch put in a
 * composer is sitting in a text field waiting for a person to press Send, and
 * saying "sent" about it would be the app claiming something nobody did.
 */
internal fun receiptDetail(batch: SentReviewBatch, nowSeconds: Long): String {
    val what =
        if (batch.placedInComposer == true) "put in ${batch.agentName}’s composer"
        else "sent to ${batch.agentName}"
    return "$what · ${handedOverWhen(batch.sentAt, nowSeconds)}"
}

/**
 * How long ago, in the shorthand the rest of this app already uses.
 *
 * An age rather than a clock time, and not by taste: a wall clock would have to
 * be formatted in some time zone, and the one honest question a receipt answers
 * is "was that this window or the last one".
 */
internal fun handedOverWhen(sentAt: Long, nowSeconds: Long): String {
    val seconds = nowSeconds - sentAt
    if (seconds < 60) return "just now"
    if (seconds < 3_600) return "${seconds / 60}m ago"
    if (seconds < 86_400) return "${seconds / 3_600}h ago"
    return "${seconds / 86_400}d ago"
}

/** `lines 12-40` → `Lines 12-40`, for a phrase written for mid-sentence use. */
internal fun capitalizedFirst(text: String): String {
    val first = text.firstOrNull() ?: return text
    return first.uppercase() + text.drop(1)
}
