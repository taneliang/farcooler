package com.farcooler.net

import com.farcooler.core.DisconnectedException
import com.farcooler.data.ReviewStorage
import com.farcooler.model.ChangeSet
import com.farcooler.model.ChangesState
import com.farcooler.model.ChangedFile
import com.farcooler.model.ChangesRow
import com.farcooler.model.DiffScope
import com.farcooler.model.FileDiffReply
import com.farcooler.model.Jump
import com.farcooler.model.ReviewBookmarks
import com.farcooler.model.ReviewCommentQueue
import com.farcooler.model.ReviewPosition
import com.farcooler.model.ReviewRef
import com.farcooler.model.Trouble
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * What a review needs from a runner.
 *
 * [Connection] implements it, and nothing else in the app does. It exists so the
 * store can be built in a JVM unit test: `ClientCore` reaches JNI and
 * `android.util.Base64` on its first line, `Connection` owns an SSH session and
 * a poll loop, and this module has no Robolectric — so a store that named either
 * of them by type could not be exercised at all, and there is no emulator for
 * this phase and no UI to look at either. Six methods, each one `changes.*` call
 * plus the send the outbox needs.
 *
 * Every method THROWS on failure rather than answering with a null. A failure is
 * not an empty diff — saying so was a real bug on the Mac once, where a runner
 * whose daemon predated this answered NOT_FOUND to everything and the pane drew
 * a worktree with no changes in it. That distinction is the whole reason
 * [ChangesState.error] exists, and it can only be made if the failure gets this
 * far.
 */
interface ChangesSource {
    suspend fun changeSet(workspace: String, fresh: Boolean): ChangeSet
    suspend fun fileDiff(workspace: String, path: String, scope: String): FileDiffReply
    suspend fun commitFiles(workspace: String, sha: String): List<ChangedFile>
    suspend fun setBase(workspace: String, baseRef: String): ChangeSet
    suspend fun markRead(workspace: String)

    /**
     * Re-read the fleet's `changes.inbox` counts.
     *
     * Not a `changes.*` call of its own — [Connection] already polls this on its
     * own cadence — but the one thing marking a worktree read actually changes.
     * See [ChangesStore.markRead].
     */
    suspend fun refreshCounts()

    /** `terminal.agent_prompt`, which is what the outbox hands a batch to. */
    suspend fun agentPrompt(terminal: String, text: String)
}

/**
 * What one worktree changed, and the diffs read so far.
 *
 * Deliberately a near-port of the Mac's `ChangesStore` and iOS's rather than a
 * fresh design: every client must agree about what "this worktree changed"
 * means, and the interesting decisions there — read a file at a time as it is
 * opened, re-read only what actually moved, never mistake a failure for an empty
 * diff — are the same decisions on this phone.
 *
 * The state itself is [ChangesState], which is pure and holds every derived
 * answer; this class holds only the I/O and the transitions. What it does NOT
 * hold is a scroll offset, and neither does the bookmark — see [ReviewPosition],
 * which explains at length why a position on this screen is a PATH. The Mac's
 * store holds an offset because its pane is destroyed when a tmux layout is
 * switched; this one's is not, since `e23718c` keeps every visited pane mounted,
 * so within one run of the app the scroll never moves and there is nothing to
 * put back.
 */
class ChangesStore(
    private val ref: ReviewRef,
    private val source: ChangesSource,
    private val storage: ReviewStorage,
    private val scope: CoroutineScope,
) {
    private val _state = MutableStateFlow(ChangesState())
    val state: StateFlow<ChangesState> = _state.asStateFlow()

    /**
     * What the reader wants to tell the agent, collected across the review.
     *
     * Its own object rather than more fields on [ChangesState], because its
     * lifetime is different in the way that matters: everything on that state is
     * derived from the daemon and can be thrown away and re-read, while a comment
     * is the only thing on this screen that a person typed and that nothing else
     * in the world has a copy of. It persists itself.
     */
    val comments = ReviewCommentQueue(
        ref = ref,
        storage = storage,
        deliver = { target, text ->
            try {
                source.agentPrompt(target.id, text)
                null
            } catch (e: Exception) {
                e.rethrowIfCancellation()
                sendTrouble(e)
            }
        },
    )

    /** Whether this store has ever read the worktree. */
    private var hasLoaded = false

    /** Whether the bookmark has already had its one chance to be offered. */
    private var hasOfferedResume = false

    /**
     * The file at the top of the screen, when none is expanded.
     *
     * Not in [ChangesState]: nothing draws it. It exists only so the bookmark has
     * something to say about a reader who was going down the list of headings
     * rather than reading a patch, which is most of the first window of a review.
     * Kept off the state for the reason iOS keeps it off its `@Published`s — this
     * moves several times a second while somebody scrolls, and a value on the
     * state would recompose the whole list on every one of them.
     */
    private var topFile: String? = null

    private var jumpSerial = 0L

    // ---- reading ----

    /**
     * Read it, but only the first time.
     *
     * The view calls this on every appearance and [load] throws away every diff
     * it holds — deliberately, since they are diffs against a base that may have
     * moved — so calling it each time would empty the screen and refetch it
     * whenever somebody glanced at another tab and came back. On this app that is
     * a chip away, so it happens more here than anywhere.
     */
    suspend fun loadIfNeeded() {
        if (hasLoaded) return
        load()
    }

    suspend fun load(fresh: Boolean = false) {
        hasLoaded = true
        _state.update { it.copy(loading = true) }
        try {
            try {
                val answer = source.changeSet(ref.workspaceId, fresh)
                _state.update { it.copy(changeSet = answer, error = null) }
            } catch (e: Exception) {
                e.rethrowIfCancellation()
                // A failure is NOT an empty diff. An old daemon is the likeliest
                // reason a phone sees this, so the message says so.
                _state.update { it.copy(changeSet = ChangeSet.EMPTY, error = loadTrouble(e)) }
            }

            // Read again rather than kept: these are diffs against a base that may
            // have just moved, and a stale hunk is worse than a missing one. The
            // generation is bumped BEFORE the commit is re-read, so that read
            // records the generation it will be checked against rather than the
            // one it replaced.
            _state.update {
                it.copy(
                    fileDiffs = emptyMap(),
                    unsupported = emptyMap(),
                    generation = it.generation + 1,
                )
            }

            // A commit's patch is immutable — it is the difference between two
            // objects that already exist — but its FILE LIST is not something
            // this store can recover on its own, and everything after this point
            // asks what is on screen. In that scope the file list IS that answer,
            // so a refresh that skipped it would fold and unfold an empty pane.
            _state.value.scope.commitSha?.let { readCommitFiles(it) }
            adoptExpansion()
            offerResumeOnce()
        } finally {
            _state.update { it.copy(loading = false) }
        }
    }

    /**
     * Read one file's diff, if it has not been read already.
     *
     * Idempotent and safe to call from a row's `LaunchedEffect`, which is exactly
     * how it is called: opening a file is what fetches it.
     */
    suspend fun ensure(path: String) {
        val before = _state.value
        if (before.isUntracked(path)) return
        if (before.fileDiffs.containsKey(path) || path in before.loadingFiles) return
        val asked = before.generation
        _state.update { it.copy(loadingFiles = it.loadingFiles + path) }
        try {
            // The sha IS the scope for a commit — see `DiffScope.wire`, which is
            // the only place that rule is spelled out.
            val diff = source.fileDiff(ref.workspaceId, path, before.scope.wire)
            // What was being compared changed while this was in flight, so these
            // lines answer a question nobody is asking any more. Stored anyway
            // they would file perfectly, under a heading that is now showing a
            // different commit — a wrong diff that looks exactly like a right one.
            if (_state.value.generation != asked) return
            _state.update { state ->
                val why = diff.unsupported
                if (why != null) {
                    state.copy(
                        unsupported = state.unsupported + (path to reason(why)),
                        fileDiffs = state.fileDiffs + (path to emptyList()),
                    )
                } else {
                    state.copy(fileDiffs = state.fileDiffs + (path to diff.lines()))
                }
            }
            prefetchAfter(path)
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            // Left unread rather than recorded as empty, so pulling to refresh
            // tries it again instead of showing a permanent blank.
        } finally {
            _state.update { it.copy(loadingFiles = it.loadingFiles - path) }
        }
    }

    /**
     * Read the file after this one, so Next lands on a patch instead of on a
     * spinner.
     *
     * Exactly ONE file ahead, and only from the file that is actually open. Since
     * a file is fetched when it is expanded and only one is ever expanded,
     * pressing Next would otherwise buy a round trip over somebody's cellular
     * link every single time — which is the tap this whole screen is built
     * around. One ahead pays for that with one extra read per file and no
     * runaway: the prefetched file is not expanded, so it does not fetch the one
     * after it, and the chain stops at one.
     */
    private fun prefetchAfter(path: String) {
        val state = _state.value
        if (path != state.expandedFile) return
        val order = state.reviewOrder
        val index = order.indexOfFirst { it.path == path }
        if (index < 0 || index + 1 >= order.size) return
        val next = order[index + 1].path
        if (state.fileDiffs.containsKey(next) || next in state.loadingFiles) return
        // Not awaited: the file on screen has landed and nothing about drawing it
        // is waiting on this. A read that arrives after the reader has moved on is
        // discarded by the generation check inside `ensure`.
        scope.launch { ensure(next) }
    }

    /**
     * Which files one commit touched, from `changes.commit_files`.
     *
     * The phone's advantage over the Mac's path, and it is a real one: that
     * client shells out to `changes files`, which has no `--json` and prints a
     * fixed-width table it has to parse back. Here the FFI hands back the same
     * `file_change_json` the change set's own files come through.
     */
    private suspend fun readCommitFiles(sha: String) {
        val asked = _state.value.generation
        try {
            val files = source.commitFiles(ref.workspaceId, sha)
            if (_state.value.generation != asked) return
            _state.update { it.copy(commitFiles = files, commitUnreadable = false) }
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            if (_state.value.generation != asked) return
            // Recorded rather than left as an empty list: a commit that could not
            // be read and a commit that changed nothing are two different things,
            // and only one of them is worth a warning triangle.
            _state.update { it.copy(commitFiles = emptyList(), commitUnreadable = true) }
        }
    }

    // ---- which comparison ----

    /**
     * Change what is being compared, and forget everything that answered the old
     * question.
     *
     * **The one place a scope changes**, which is this platform's answer to
     * iOS's `didSet`. Swift gets the reset for free on every assignment; Kotlin
     * has no such hook, so [ChangesState.scope] is read-only from outside and
     * every road into it — the segmented control, picking a commit, the resume —
     * comes through here. A public `var` would have been a way to move the scope
     * without dropping the diffs, and the bug that produces is one commit's
     * hunks under another commit's subject.
     */
    private fun changeScope(next: DiffScope) {
        val before = _state.value
        if (before.scope == next) return
        _state.update {
            it.copy(
                scope = next,
                // The diffs on hand answer the OTHER question. Kept, they would
                // show a committed patch under an "Uncommitted" heading — or,
                // now that a scope carries its sha, one commit's patch under
                // another commit's subject, which is the same bug wearing a
                // better disguise.
                fileDiffs = emptyMap(),
                unsupported = emptyMap(),
                // Every scope is a different file list, so the open file goes
                // with it rather than being carried across — the same path can
                // exist in both and mean two different patches.
                expandedFile = null,
                // A commit's file list is FETCHED rather than derived from the
                // change set, so leaving the previous one in place would put the
                // old commit's files under the new one's subject for as long as
                // the round trip takes — which on a phone link is long enough to
                // read.
                commitFiles = emptyList(),
                commitUnreadable = false,
                generation = it.generation + 1,
            )
        }
        // The row that was at the top belongs to the list being replaced. A path
        // can exist in both scopes, so a leftover here would let a card nobody
        // can see decide where the bookmark says the reader is.
        topFile = null
        adoptExpansion()
        rememberPosition()
    }

    /**
     * Back to the whole branch: merge base to HEAD, every commit at once.
     *
     * No round trip. The change set never stopped being held — only the commit's
     * patches have to go.
     */
    fun showWholeBranch() = changeScope(DiffScope.Branch)

    fun showUncommitted() = changeScope(DiffScope.Local)

    /** Show one commit, against its first parent. */
    suspend fun select(sha: String) {
        // Already showing it. Re-reading would spend a round trip to arrive at
        // the same list and would take every file the reader had opened with it.
        if (_state.value.scope == DiffScope.Commit(sha)) return
        changeScope(DiffScope.Commit(sha))
        val asked = _state.value.generation
        // The same flag a full load raises, and for the same reason: this scope's
        // file list is empty until the call answers, and an empty list with
        // nothing to say about why is a pane asserting that the commit changed
        // nothing.
        _state.update { it.copy(loading = true) }
        readCommitFiles(sha)
        // Lowered by hand rather than in a `finally`, because a `finally` would
        // lower it for a selection that has already been superseded — two commits
        // chosen in quick succession finish in the order the daemon answers, not
        // the order they were asked for. The later one owns the spinner, and
        // clearing it here would blink "nothing changed" over its list on the way
        // past.
        if (_state.value.generation != asked) return
        _state.update { it.copy(loading = false) }
        adoptExpansion()
    }

    /**
     * Begin the itinerary: the first commit the branch made.
     *
     * The way in that a history sheet is not. A sheet is the right shape for
     * "which one of these", and the wrong shape for "start at the beginning and
     * keep going" — which for an agent-authored branch is the reading that makes
     * it legible, and the reading this screen exists to support.
     */
    suspend fun startAtFirstCommit() {
        val first = _state.value.commitsInOrder.firstOrNull() ?: return
        select(first.sha)
    }

    /**
     * Move one commit along the branch, without going back out to a sheet.
     *
     * This is what makes commit-by-commit a path rather than a detour: the reader
     * finishes a commit's last file and the same control that has been moving
     * them through files moves them to the next intention. Returning to a picker
     * between every commit is twelve extra taps on a branch of twelve and a reason
     * not to read it this way at all.
     */
    suspend fun showNextCommit() {
        select(_state.value.nextCommit?.sha ?: return)
    }

    suspend fun showPreviousCommit() {
        select(_state.value.previousCommit?.sha ?: return)
    }

    // ---- moving through the files ----

    /** Open one file, closing whatever was open. */
    fun expand(path: String?) {
        if (_state.value.expandedFile == path) return
        _state.update { it.copy(expandedFile = path) }
        rememberPosition()
        if (path != null) jumpTo(path)
    }

    fun toggle(path: String) {
        if (_state.value.expandedFile == path) {
            _state.update { it.copy(expandedFile = null) }
            rememberPosition()
        } else {
            expand(path)
        }
    }

    /**
     * The next file, or the first one when the reader is still on the list.
     *
     * Starting the sequence from "nothing open" is the point: Next is then the
     * single control that begins a review as well as continuing one, which on a
     * phone held in one hand is worth more than the symmetry of having it do
     * nothing until something is selected.
     */
    fun showNextFile() {
        val state = _state.value
        val order = state.reviewOrder
        if (order.isEmpty()) return
        val index = state.currentIndex ?: return expand(order[0].path)
        if (index + 1 >= order.size) return
        expand(order[index + 1].path)
    }

    fun showPreviousFile() {
        val state = _state.value
        val index = state.currentIndex ?: return
        if (index <= 0) return
        expand(state.reviewOrder[index - 1].path)
    }

    /**
     * Keep the open file open, unless it has stopped existing.
     *
     * Not "collapse everything on load": a poll that picks up one new commit
     * would then fold away the file somebody is mid-way through reading, which on
     * this screen is the whole of what they were doing. Only a file that is no
     * longer in this scope's list gives way, and it has to — nothing would draw
     * it.
     */
    private fun adoptExpansion() {
        val state = _state.value
        val open = state.expandedFile ?: return
        if (state.files.any { it.path == open }) return
        _state.update { it.copy(expandedFile = null) }
        rememberPosition()
    }

    // ---- scrolling ----

    /** Raise a jump. See [Jump] for why it carries a serial. */
    private fun jumpTo(key: String) {
        jumpSerial += 1
        val serial = jumpSerial
        _state.update { it.copy(jump = Jump(serial, key)) }
    }

    /**
     * The view acted on a jump.
     *
     * By serial, so a jump raised while the scroll animation was running is not
     * swallowed by the clear that belongs to the previous one.
     */
    fun clearJump(serial: Long) {
        _state.update { if (it.jump?.serial == serial) it.copy(jump = null) else it }
    }

    /**
     * The topmost file row on screen changed.
     *
     * Fed by the view from `LazyListState.layoutInfo` through
     * [com.farcooler.model.ReviewScroll.topVisible], which is where the
     * rebuilt-for-Compose half of this lives. Nothing is published: this fires
     * several times a second while somebody scrolls, and a value on
     * [ChangesState] would re-evaluate a forty-row lazy list on every one of
     * them.
     */
    fun noteTopFile(path: String?) {
        if (path == topFile) return
        topFile = path
        rememberPosition()
    }

    // ---- where you stopped ----

    /**
     * Write down where the reader is.
     *
     * Called on every move rather than on the way out, because there is no way
     * out to hook: Android kills a backgrounded process without telling it,
     * which in the situation this screen is built for is the NORMAL way a review
     * window ends. The record is three short strings and the write is `apply`.
     */
    private fun rememberPosition() {
        // Not while an offer is on the table. The first rows realize themselves
        // the moment the list draws, and each one reports its visibility — so
        // without this, arriving at the screen would overwrite the very position
        // the resume card is offering, and a reader who tapped it after a second
        // process death would be taken to the top of the branch they had just
        // opened.
        val state = _state.value
        if (state.resume != null) return
        ReviewBookmarks.write(
            storage,
            ref,
            ReviewPosition(
                scope = state.scope.wire,
                file = state.expandedFile,
                topFile = topFile,
                savedAt = System.currentTimeMillis() / 1_000,
            ),
        )
    }

    /**
     * Read the bookmark, once per run of the app, and offer it.
     *
     * After the first load rather than before it, because the offer names a
     * commit's subject and a file, and both are things only the change set can
     * supply. Offered once: a pull to refresh is somebody asking about the branch
     * they are already reading, and re-offering to take them somewhere else would
     * be the app arguing with them.
     */
    private fun offerResumeOnce() {
        if (hasOfferedResume) return
        hasOfferedResume = true
        val state = _state.value
        if (state.error != null) return
        val saved = ReviewBookmarks.read(storage, ref) ?: return
        if (!saved.isSomewhere) return
        // Already there. Two ways that happens and both must be caught, or the
        // card is an interruption that resolves to nothing:
        //
        // - the store outlived a tab switch rather than a process death, so every
        //   part of the position still matches what is on screen; or
        // - the position IS the top of the list — somebody who opened the tab,
        //   read the first heading and got called away is not somewhere that
        //   needs restoring to.
        val unmoved = saved.scope == state.scope.wire &&
            saved.file == state.expandedFile && saved.topFile == topFile
        val atTheTop = saved.scope == state.scope.wire && saved.file == null &&
            saved.topFile == state.reviewOrder.firstOrNull()?.path
        if (unmoved || atTheTop) return
        _state.update { it.copy(resume = saved) }
    }

    /**
     * Take the offer.
     *
     * Every step is allowed to fail, and each failure lands one level further out
     * with a sentence saying which one gave way. That is the whole contract of
     * this feature: a bookmark is a hint about a branch that an agent has probably
     * kept editing, and the alternative to landing nearby and saying so is either
     * lying about where you are or refusing to move.
     */
    suspend fun applyResume() {
        val saved = _state.value.resume ?: return
        _state.update { it.copy(resume = null, resumeNote = null) }

        val sha = ReviewPosition.sha(saved.scope)
        if (sha != null) {
            if (_state.value.changeSet.commits.none { it.sha == sha }) {
                // The commit was amended or rebased away overnight, which for an
                // agent-authored branch is not an edge case. The branch as a whole
                // still contains its work, so that is where this lands.
                _state.update {
                    it.copy(
                        resumeNote = "That commit isn’t on the branch anymore — it was " +
                            "amended or rebased. This is the whole branch instead."
                    )
                }
                showWholeBranch()
                jumpTo(ChangesRow.TOP_KEY)
                return
            }
            select(sha)
        } else if (saved.scope == DiffScope.Local.wire) {
            showUncommitted()
        } else {
            showWholeBranch()
        }

        val live = _state.value.files.map { it.path }
        val file = saved.file
        if (file != null) {
            if (file !in live) {
                _state.update {
                    it.copy(
                        resumeNote = "${file.substringAfterLast('/')} isn’t in this diff " +
                            "anymore, so this is the top."
                    )
                }
                jumpTo(ChangesRow.TOP_KEY)
                return
            }
            expand(file)
        } else {
            val top = saved.topFile
            if (top != null && top in live) jumpTo(top)
        }
    }

    fun dismissResume() {
        // Not forgotten, only declined. Somebody who taps the X wants this out of
        // the way, not a bookmark deleted — the next window may well be the one
        // they meant to resume in.
        _state.update { it.copy(resume = null, resumeNote = null) }
    }

    fun clearResumeNote() {
        _state.update { it.copy(resumeNote = null) }
    }

    // ---- the two writes ----

    /**
     * Mark this worktree as read, which is what clears its badge everywhere.
     *
     * **It does not reload the diff, and that is deliberate.** iOS calls `load()`
     * afterwards, which throws away every patch already fetched and folds the
     * file being read; `92058f4` found and fixed the same shape on the Mac, where
     * `markRead` called a `load()` that also ran `reset()` — "the one gesture
     * meaning 'I have finished reading this' would have dropped every fetched
     * patch". Marking read changes nothing about the diff. What it changes is the
     * inbox, so that is what is refreshed, and directly rather than waiting out a
     * poll, because a tap has to answer for itself.
     *
     * Failures are swallowed. The badge is a decoration on rows that are correct
     * without it, and there is nothing for a person to do about a `mark_read`
     * that did not land except tap it again.
     */
    suspend fun markRead() {
        attempt { source.markRead(ref.workspaceId) }
        attempt { source.refreshCounts() }
    }

    /**
     * Pin what this worktree is compared against.
     *
     * The affordance that exists because a GUESSED base produces a wrong diff
     * that looks exactly like a right one — see [ChangeSet.baseIsGuessed]. The
     * answer is a whole new change set, so everything read against the old base
     * goes with it.
     */
    suspend fun setBase(baseRef: String) {
        _state.update { it.copy(loading = true) }
        try {
            val answer = source.setBase(ref.workspaceId, baseRef)
            _state.update {
                it.copy(
                    changeSet = answer,
                    error = null,
                    fileDiffs = emptyMap(),
                    unsupported = emptyMap(),
                    commitFiles = emptyList(),
                    commitUnreadable = false,
                    generation = it.generation + 1,
                )
            }
            _state.value.scope.commitSha?.let { readCommitFiles(it) }
            adoptExpansion()
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            _state.update { it.copy(error = loadTrouble(e)) }
        } finally {
            _state.update { it.copy(loading = false) }
        }
    }

    companion object {
        /** Why the daemon would not render a patch, in words. */
        internal fun reason(code: String): String = when (code) {
            "binary" -> "Binary file"
            "submodule" -> "Submodule"
            "combined_diff" -> "A merge commit, shown against its first parent"
            else -> "This patch couldn’t be read"
        }

        /**
         * The core's answer, as something worth putting on a phone screen.
         *
         * The old-runner arm names the cause, so it passes no transcript: a dump
         * of what the call said, under a sentence that already answers the
         * question, is noise. That is the same scoping the Mac makes with its
         * `if !old`, and it is why [Trouble] has two fields instead of always
         * filling both.
         *
         * The wording is the phone's rather than the Mac's, which ends "Update it
         * in Settings › Runners." A phone cannot update a runner, so carrying that
         * over would send somebody looking for a screen that does not exist.
         */
        internal fun loadTrouble(e: Exception): Trouble {
            val text = (e.message ?: "").lowercase()
            if (text.contains("not found") || text.contains("unknown method")) {
                return Trouble("This runner’s Far Cooler is too old to review changes.")
            }
            return Trouble(
                "Couldn’t read this workspace. The request that reads it didn’t finish.",
                e.message,
            )
        }

        /**
         * Why a batch of notes did not go.
         *
         * Never the raw error AS the sentence: what the core hands back is a Rust
         * word for an empty session slot, and the person reading it wanted to send
         * a sentence to an agent. That was never a reason to drop it either — the
         * first two arms know what happened and say so, and the third knows
         * nothing, so its words travel in the box.
         */
        internal fun sendTrouble(e: Exception): Trouble {
            if (e is DisconnectedException) {
                return Trouble(
                    "The connection to this runner dropped, so these are still here. " +
                        "Try again once it’s back."
                )
            }
            val text = (e.message ?: "").lowercase()
            if (text.contains("not found") || text.contains("unknown method")) {
                return Trouble(
                    "That pane isn’t running an agent anymore, so there was nothing to send to."
                )
            }
            return Trouble("Couldn’t send these. They’re still here.", e.message)
        }
    }
}

/**
 * The stores, kept alive past the screens that show them.
 *
 * The Changes tab is one pane among several, and although `e23718c` keeps every
 * visited pane MOUNTED — so a store held in a `remember` would survive a tab
 * switch — it does not keep them mounted forever: three panes, least recently
 * shown evicted, and the whole workspace goes when the back stack does. A store
 * rebuilt from nothing means every fold reopened and every diff re-fetched over
 * somebody's cellular link, which is disruptive in exactly the case the tab
 * exists for.
 *
 * ## Keyed by workspace, and owned by the runner
 *
 * A map keyed by workspace id ALONE would be a collision, and not a theoretical
 * one: ids are minted per daemon, this app connects to every runner at once, and
 * `df87410` had to answer exactly this for the front door. The answer here is
 * that the host half of the key is structural — one of these belongs to one
 * [Connection], so the only way to reach a store is through the runner that owns
 * it, and there is no call that could name the wrong one. Every store still
 * carries its [ReviewRef] so that what it writes to disk — a bookmark, an unsent
 * note — is keyed `host/workspace` the way [ReviewRef] argues it must be.
 *
 * Held for the lifetime of the connection: a handful of change sets is small next
 * to re-reading one over a phone link, and a connection going away is the point
 * at which none of them mean anything anyway.
 */
class ChangesStores(
    private val hostId: String,
    private val source: ChangesSource,
    private val storage: ReviewStorage,
    private val scope: CoroutineScope,
) {
    private val stores = mutableMapOf<String, ChangesStore>()

    @Synchronized
    fun store(workspaceId: String): ChangesStore = stores.getOrPut(workspaceId) {
        ChangesStore(ReviewRef(hostId, workspaceId), source, storage, scope)
    }
}
