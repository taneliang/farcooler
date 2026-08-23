package com.farcooler.model

// Everything the Changes tab draws, as one value.
//
// **This is where the port stops transliterating, and the reason is Compose.**
// iOS's `ChangesStore` is a dozen `@Published` properties and SwiftUI diffs the
// resulting view tree, so a `didSet` that assigns six of them in sequence is
// invisible: nothing is drawn until the run loop comes back round. A dozen
// `StateFlow`s collected separately in a composable are a dozen independent
// recomposition triggers over one logical change, and — worse — the frames
// between them are real. Switching scope clears the file diffs, the unsupported
// map, the expanded file, the commit list and the generation; a recomposition
// landing halfway through that would draw one commit's hunks under another
// commit's heading, which is the exact bug `scope`'s `didSet` exists to
// prevent.
//
// So the store holds ONE `StateFlow<ChangesState>` and every transition is a
// single `copy`. What that buys beyond atomicity is that all of the arithmetic
// below is a pure function of an immutable value: `ChangesStateTest` proves the
// scope switch, the generated split, the reading order and `7 of 23` with no
// coroutines, no core, and no device.

/**
 * A scroll the view should perform, once.
 *
 * ## Not a path, and this is the first of the two things phase 5 could not port
 *
 * iOS names a target with `proxy.scrollTo(path)` — SwiftUI resolves a `.id` to
 * a position for it. **Compose has no `scrollTo(id)`.** `LazyListState` moves by
 * INDEX (`animateScrollToItem`), and an index into a lazy list is only knowable
 * from the list itself. Deriving it in the view would put the layout in two
 * places — the `LazyColumn`'s item list and whatever the scroll code believed
 * it to be — and a diff that has an extra heading in it exactly when a branch
 * regenerated a lockfile is the worst possible thing to be wrong about by one.
 *
 * The fix is that the model owns the layout. [ChangesState.rows] IS the item
 * list, [ChangesState.indexOf] resolves a key against it, and the view does
 * `rows` and `animateScrollToItem` from the same value in the same
 * recomposition. A key is still a path, so everything [ReviewPosition] argues
 * about anchors survives intact.
 *
 * [serial] is what makes it fire twice. Tapping the same file in the index
 * twice, or pressing Next into a file that is already open after a refresh
 * reshuffled the list, has to move the scroll both times, and a bare key
 * compares equal and does nothing the second time. The view clears it by serial
 * after acting, so nothing re-scrolls under somebody who has since scrolled
 * away and a jump raised meanwhile is not swallowed.
 */
data class Jump(val serial: Long, val key: String)

/**
 * One item of the Changes list, in the order it is drawn.
 *
 * [key] is what a `LazyColumn` keys on and what a [Jump] names, and the two
 * being one string is the point — see [Jump].
 */
sealed interface ChangesRow {
    val key: String

    /**
     * Everything above the files: the summary card, the resume offer, and
     * whichever notice applies.
     *
     * One row rather than several because it is one block that is either at the
     * top of the list or scrolled past, and because "land at the top and say
     * why" needs somewhere to land. Its key is a string no path can be.
     */
    data object Summary : ChangesRow {
        override val key: String get() = TOP_KEY
    }

    /**
     * The line that separates what somebody wrote from what a tool wrote.
     *
     * Present whenever anything generated is, including on the branch that is
     * ALL lockfile — which is iOS's rule and is the right one: a list of four
     * files with no heading, whose counts do not match the summary above it, is
     * the confusion the split exists to remove.
     */
    data class GeneratedHeading(val count: Int, val lines: Int) : ChangesRow {
        override val key: String get() = GENERATED_KEY
    }

    data class File(val file: ChangedFile) : ChangesRow {
        override val key: String get() = file.path
    }

    companion object {
        const val TOP_KEY = "changes.top"
        const val GENERATED_KEY = "changes.generated"
    }
}

/**
 * What one worktree changed, and everything read about it so far.
 *
 * Deliberately a near-port of the Mac's and iOS's stores rather than a fresh
 * design: every app must agree about what "this worktree changed" means, and
 * the interesting decisions there — read a file at a time as it scrolls into
 * view, never mistake a failure for an empty diff, hold exactly one file open —
 * are the same decisions on this phone.
 */
data class ChangesState(
    val changeSet: ChangeSet = ChangeSet.EMPTY,
    val scope: DiffScope = DiffScope.Branch,
    /**
     * What the commit on screen touched, from `changes.commit_files`.
     *
     * A separate call and not a filter over the branch's files, because the
     * change set carries per-file counts for the WHOLE branch and nothing
     * per-commit, on purpose: a branch that regenerated a lockfile touches
     * thousands of files, and shipping that per commit to draw a list is the
     * cost this product does not pay — least of all over somebody's cellular
     * link.
     */
    val commitFiles: List<ChangedFile> = emptyList(),
    /**
     * Set when that call failed, so the pane can say WHICH nothing it is
     * showing. A commit can genuinely stop being readable while somebody is
     * looking at it — an amend or a rebase rewrites the branch underneath — and
     * "nothing changed here" is the wrong sentence for a commit that could not
     * be read at all.
     */
    val commitUnreadable: Boolean = false,
    /**
     * The one file whose patch is open, or null when the pane is a list of
     * headings.
     *
     * ONE, not a set, and that is a change of shape rather than a tightening of
     * a limit. Every file starts folded so the pane opens as a list of what
     * changed rather than as the first file's patch, and on a phone one
     * expanded diff already fills the screen.
     *
     * Holding exactly one is also what makes the rest of this screen possible.
     * "Where am I" has an answer, so the bar at the bottom can say `7 of 23`;
     * "the next file" has an answer, so Next and Previous can exist at all; and
     * the bookmark has one path to write down instead of a set whose meaning on
     * re-entry would be a guess.
     */
    val expandedFile: String? = null,
    /**
     * Every file's patch, by path, as it is read.
     *
     * Filled a file at a time as each is opened rather than up front: a branch
     * that touched forty files would otherwise pay forty round trips before
     * drawing a screenful — and on a phone those round trips are over somebody's
     * cellular link.
     */
    val fileDiffs: Map<String, List<DiffComputation.Line>> = emptyMap(),
    /** Files the daemon would not render, and why. */
    val unsupported: Map<String, String> = emptyMap(),
    /**
     * Files being read right now, so a row can say so rather than look empty.
     * An unread file and a file with no hunks are not the same thing.
     */
    val loadingFiles: Set<String> = emptySet(),
    val loading: Boolean = false,
    /**
     * Why this worktree is not showing a diff: a sentence this app wrote, and —
     * only where it has no account of its own — the runner's own words.
     *
     * [Trouble] rather than one string, the same two fields `AgentStream` and
     * the review outbox carry. A failure nobody can diagnose is a failure
     * nobody can fix, and a raw core error is never the sentence.
     */
    val error: Trouble? = null,
    /** A bookmark from a previous run, offered but not applied. See [ReviewPosition]. */
    val resume: ReviewPosition? = null,
    /**
     * Why the resume did not land exactly where it was aimed.
     *
     * A position is a HINT and is allowed to be wrong: an agent that kept
     * working overnight deletes files, rewrites commits, and renames the thing
     * that was being read. The rule is to land as close as possible — the top of
     * the branch, the top of the file — and to say so, rather than to pretend
     * the bookmark was honored or to refuse to move at all.
     */
    val resumeNote: String? = null,
    val jump: Jump? = null,
    /**
     * Bumped every time the cached diffs are thrown away.
     *
     * A read in flight when the reader moves on must not be filed under what
     * they moved to. Every `changes.file_diff` and `changes.commit_files`
     * answers with a well-formed result, and the stale one is indistinguishable
     * from the fresh one once it has landed — a patch keyed on a path alone
     * slots perfectly under a heading that is now showing a different commit. So
     * every read records the generation it started in and compares before
     * storing anything.
     */
    val generation: Int = 0,
) {
    // ---- which files this scope is about ----

    /**
     * The files this scope is about.
     *
     * Branch is what the branch COMMITTED — the same thing the summary card
     * counts, but no longer the same thing the FLEET counts: `7927c13` moved
     * `change_set::shortstat` to compare the base against the WORKING TREE and
     * taught it to count untracked lines, so the `+N −M` on a dirty worktree's
     * row in the fleet list and on the front door reads higher than this one.
     *
     * Local is what has not been committed yet, and it has to come from the
     * working tree rather than from the committed file list, or a file an agent
     * just wrote — the only kind of file Local exists to show — would never
     * appear in it.
     *
     * Commit is one commit against its first parent, and both halves of the
     * screen come from that same comparison, so the file list and the patches
     * under it cannot disagree about what the commit did.
     */
    val files: List<ChangedFile>
        get() = when (scope) {
            is DiffScope.Branch -> changeSet.files
            is DiffScope.Commit -> commitFiles
            is DiffScope.Local -> localFiles()
        }

    /**
     * The dirty files with their counts.
     *
     * Read from the working tree the daemon sent, not counted from the diffs on
     * screen. Counting the hunks was right about COST and wrong about
     * correctness, and the Mac carried the same defect until `24f2c1d`: a
     * per-file round trip to restate what is already drawn would be absurd, but
     * the hunks only hold the number once they have been fetched, and
     * [fileDiffs] fills lazily. So every count started at zero and arrived late,
     * and the card's total would have climbed as the reader scrolled.
     *
     * Grouped once rather than filtered per path, which is where this differs
     * from iOS in cost and not in answer: that loop is O(paths × records), and a
     * worktree with four hundred dirty files is exactly the one where it is
     * called on every recomposition.
     */
    private fun localFiles(): List<ChangedFile> {
        val tree = changeSet.workingTree ?: return emptyList()
        // Summed rather than found, because a file that is staged and then
        // modified again is in both groups with its own diff in each.
        val byPath = (tree.changes ?: emptyList()).groupBy { it.path }
        return dirtyPaths.map { path ->
            val records = byPath[path] ?: emptyList()
            ChangedFile(
                path = path,
                statusWord = localStatus(path).wire,
                oldPath = null,
                insertions = records.sumOf { it.insertions },
                deletions = records.sumOf { it.deletions },
                binary = records.any { it.binary },
            )
        }
    }

    /**
     * Every path git calls dirty, in one order: staged, then unstaged, then
     * conflicted, then untracked — and never twice, because a file can be
     * staged and modified again and it is still one file.
     */
    private val dirtyPaths: List<String>
        get() {
            val tree = changeSet.workingTree ?: return emptyList()
            return (tree.staged + tree.unstaged + tree.conflicted + tree.untracked).distinct()
        }

    private fun localStatus(path: String): ChangedFileStatus {
        val tree = changeSet.workingTree ?: return ChangedFileStatus.MODIFIED
        if (path in tree.conflicted) return ChangedFileStatus.CONFLICTED
        if (path in tree.untracked) return ChangedFileStatus.UNTRACKED
        return tree.changes?.firstOrNull { it.path == path }?.status
            ?: ChangedFileStatus.MODIFIED
    }

    /**
     * Whether git has never seen this file.
     *
     * It has no diff and cannot be given one — `git diff` compares against
     * something recorded, and nothing is recorded for a file only just written.
     * It still belongs in the list, since a file an agent created is the most
     * interesting thing in a local change set, so the row says which kind of
     * nothing it is showing.
     */
    fun isUntracked(path: String): Boolean {
        // Nothing inside a commit is untracked — committing is what tracking IS
        // — and the working tree's list is about right now, not about then. Left
        // to speak here it would refuse to fetch the diff of a file some commit
        // changed and somebody has since deleted and rewritten.
        if (scope.commitSha != null) return false
        return changeSet.workingTree?.untracked?.contains(path) == true
    }

    // ---- what a tool wrote ----

    /**
     * Split rather than filtered: a lockfile that moved is a real fact about a
     * branch, and hiding it would be this screen deciding what the reader is
     * allowed to see. Only its SIZE is misleading, so the two are counted apart
     * and both are listed.
     */
    val generatedFiles: List<ChangedFile> get() = files.filter { it.isGenerated }
    val handWrittenFiles: List<ChangedFile> get() = files.filterNot { it.isGenerated }

    /**
     * The files in reading order: what somebody wrote, then what a tool wrote.
     *
     * The order the screen draws AND the order Next and Previous walk, which is
     * why it is one property rather than a sort inside the view: `7 of 23` has
     * to count the list the reader is looking at, and a display order that
     * differed from the navigation order would make Next jump backwards up the
     * screen.
     *
     * Through [GeneratedFile.reviewOrder] rather than `handWrittenFiles +
     * generatedFiles`, which is the same list by a different route and is not a
     * tidy-up: every client walks this order now, and a partition written twice
     * is two answers to "which file does Next open" waiting to differ.
     */
    val reviewOrder: List<ChangedFile> get() = GeneratedFile.reviewOrder(files) { it.path }

    /** `4,012 lines` across the generated files, for the line under the counts. */
    val generatedLineCount: Int get() = generatedFiles.sumOf { it.insertions + it.deletions }

    val writtenInsertions: Int get() = handWrittenFiles.sumOf { it.insertions }
    val writtenDeletions: Int get() = handWrittenFiles.sumOf { it.deletions }
    val generatedInsertions: Int get() = generatedFiles.sumOf { it.insertions }
    val generatedDeletions: Int get() = generatedFiles.sumOf { it.deletions }

    // ---- the rows ----

    /**
     * The list a `LazyColumn` draws, and the list an index into it means.
     *
     * See [Jump] for why the model owns this rather than the view.
     */
    val rows: List<ChangesRow>
        get() {
            val order = reviewOrder
            val out = ArrayList<ChangesRow>(order.size + 2)
            out.add(ChangesRow.Summary)
            var headed = false
            for (file in order) {
                if (!headed && file.isGenerated) {
                    // Everything from here on is generated: `reviewOrder` is a
                    // stable partition, so the first one is the boundary.
                    val generated = order.subList(out.size - 1, order.size)
                    out.add(
                        ChangesRow.GeneratedHeading(
                            count = generated.size,
                            lines = generated.sumOf { it.insertions + it.deletions },
                        )
                    )
                    headed = true
                }
                out.add(ChangesRow.File(file))
            }
            return out
        }

    /** Where a [Jump]'s key sits in [rows], or null when it is no longer there. */
    fun indexOf(key: String): Int? = rows.indexOfFirst { it.key == key }.takeIf { it >= 0 }

    // ---- moving through the files ----

    val currentIndex: Int?
        get() {
            val open = expandedFile ?: return null
            return reviewOrder.indexOfFirst { it.path == open }.takeIf { it >= 0 }
        }

    /**
     * `7 of 23`, or the count on its own when nothing is open.
     *
     * Said in words rather than left to a scrollbar, because a phone's scrollbar
     * appears while you drag and disappears while you read — so the one question
     * a long diff raises, "how much of this is left", has no answer on screen at
     * the moment it is asked.
     */
    val positionLabel: String
        get() {
            val total = reviewOrder.size
            if (total == 0) return "No files"
            val index = currentIndex ?: return if (total == 1) "1 file" else "$total files"
            return "${index + 1} of $total"
        }

    fun isExpanded(path: String): Boolean = expandedFile == path

    val hasNextFile: Boolean
        get() {
            if (reviewOrder.isEmpty()) return false
            val index = currentIndex ?: return true
            return index + 1 < reviewOrder.size
        }

    val hasPreviousFile: Boolean get() = (currentIndex ?: 0) > 0

    // ---- one commit at a time ----

    /**
     * The commits in the order they are READ, which is the order they were made:
     * base forward.
     *
     * Working THROUGH a branch is a different activity from picking one commit
     * out of it. Each commit is one intention, and an agent's intentions only
     * make sense forwards — the third commit fixes what the second introduced,
     * and read backwards it is a repair to something that has not happened yet.
     * `commits_since` already logs with `--reverse` for exactly this reading, so
     * this is the wire's own order.
     */
    val commitsInOrder: List<ChangeCommit> get() = changeSet.commits

    /**
     * The commits this branch made, newest first — the picker's order.
     *
     * The opposite of [commitsInOrder], and both are right for what they are
     * for: a picker is reached for with one commit in mind and it is almost
     * always the newest.
     */
    val commitsNewestFirst: List<ChangeCommit> get() = changeSet.commits.reversed()

    /**
     * What the change set still knows about the commit on screen, if it knows
     * anything.
     *
     * Null for a commit that has left the branch, which an amend or a rebase
     * mid-review does exactly — while the diff itself usually stays readable,
     * because the object it names is still in the repository until git prunes
     * it. So this going null is not an error; it is the one moment the header has
     * to stop claiming a subject and an author it can no longer support.
     */
    val selectedCommitInfo: ChangeCommit?
        get() {
            val sha = scope.commitSha ?: return null
            return changeSet.commits.firstOrNull { it.sha == sha }
        }

    val commitIndex: Int?
        get() {
            val sha = scope.commitSha ?: return null
            return commitsInOrder.indexOfFirst { it.sha == sha }.takeIf { it >= 0 }
        }

    /**
     * `Commit 3 of 12`, for the header of a commit being read in sequence.
     *
     * Null for a commit the branch no longer lists, which an amend mid-review
     * produces: it has no position in a sequence it is not in, and inventing one
     * would be the header's one claim that could be flatly wrong.
     */
    val commitPositionLabel: String?
        get() {
            val index = commitIndex ?: return null
            return "Commit ${index + 1} of ${commitsInOrder.size}"
        }

    val nextCommit: ChangeCommit?
        get() {
            val index = commitIndex ?: return null
            return commitsInOrder.getOrNull(index + 1)
        }

    val previousCommit: ChangeCommit?
        get() {
            val index = commitIndex ?: return null
            return if (index > 0) commitsInOrder[index - 1] else null
        }

    /**
     * Whether Next should carry on into the following commit.
     *
     * Only at the END of a commit's files, so the control means one thing at a
     * time: while there are files left it is "next file", and exactly once per
     * commit it becomes "next commit" and says so on its face.
     */
    val nextIsCommit: Boolean
        get() = scope.commitSha != null && !hasNextFile && nextCommit != null

    /** `+N` for the commit on screen, summed from its own file list. */
    val commitInsertions: Int get() = commitFiles.sumOf { it.insertions }
    val commitDeletions: Int get() = commitFiles.sumOf { it.deletions }

    /**
     * The two numbers for the commit on screen, generated files held apart.
     *
     * Three sources in order of how much they know, which is not
     * over-engineering but the three things that are actually true at different
     * moments:
     *
     * 1. When this comparison contains a generated file, the headline is the
     *    hand-written subtotal — summed from the file list, because the row's own
     *    `--shortstat` counts the whole commit and the lockfile in it is the
     *    number this split exists to stop showing.
     * 2. Otherwise the daemon's own count off [ChangeCommit], which arrives with
     *    the change set and is therefore right before the commit's file list has
     *    landed.
     * 3. Otherwise the sum of that file list, which is what every client did for
     *    as long as the daemon's three count fields were hardcoded zeroes.
     *
     * All three are the same number when more than one is available:
     * `--shortstat` is the sum of the same commit's `--numstat`, with renames
     * detected on both sides and binaries contributing zero to both.
     */
    val commitCounts: Pair<Int, Int>?
        get() {
            if (generatedFiles.isNotEmpty()) {
                if (commitFiles.isEmpty()) return null
                return writtenInsertions to writtenDeletions
            }
            selectedCommitInfo?.counts?.let { return it }
            if (commitFiles.isEmpty()) return null
            return commitInsertions to commitDeletions
        }

    /**
     * `+N −M` for everything uncommitted: the Uncommitted segment's total.
     *
     * The working tree's own records, which `change_set::apply_uncommitted_counts`
     * fills in the response this screen already read — never a sum of the diffs
     * on screen, which arrive one row at a time and would make this header climb
     * while it was being looked at.
     *
     * Not [ChangeSet.insertions], which is the BRANCH — `numstat(base, HEAD)`,
     * committed work only. The two answer different questions and are shown
     * under different segments.
     *
     * A file that is staged and then edited again has a record in both groups and
     * both are counted. That is exact for every file in one group or the other,
     * and an upper bound only when the same lines are touched twice.
     */
    val uncommittedInsertions: Int get() = uncommittedFiles.sumOf { it.insertions }
    val uncommittedDeletions: Int get() = uncommittedFiles.sumOf { it.deletions }

    /**
     * Every count record the working tree carries, staged and unstaged and
     * untracked alike. A conflicted path has none — git gives it its own group
     * and `--numstat` prints two records for it, so the daemon files it nowhere
     * rather than guessing which side to believe.
     */
    private val uncommittedFiles: List<ChangedFile>
        get() = changeSet.workingTree?.changes ?: emptyList()
}
