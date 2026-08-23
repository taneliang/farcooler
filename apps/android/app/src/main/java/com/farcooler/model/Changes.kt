package com.farcooler.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// What a worktree changed, on a phone.
//
// The shapes below decode what one Rust function emits —
// `change_set_json`/`file_change_json`/`file_diff_json` in
// `crates/client/src/changes_json.rs` — which is the same function the Mac's
// `ChangesModel.swift` and iOS's `Changes.swift` decode. There is one
// definition of what a change set looks like on the wire and three readers of
// it, not three definitions. `ChangesDecodeTest` transcribes those `json!`
// blocks key for key so a rename on either end fails a test here rather than
// going quiet on a phone as a permanently-zero count.
//
// **Two casings out of one FFI, and it is not a mistake.** `change_set_json`
// builds `base_ref` / `old_path` / `working_tree`; `file_diff_json` builds
// `oldStart` / `newNumber` / `firstParentOfMerge`. Both are decoded with
// `ignoreUnknownKeys = true`, so a missing `@SerialName` is a field that stays
// at its default forever rather than an error anybody would notice — the same
// trap `InboxRow` in `Model.kt` records for `changes.inbox`.
//
// Nothing here computes state. The daemon derives the diff, the counts, and
// which files are dirty; this draws what it derived, the same contract the
// terminal rows keep.

@Serializable
data class ChangeSet(
    val branch: String = "",
    @SerialName("base_ref") val baseRef: String = "",
    /**
     * Where the base came from: `recorded`, `upstream`, `guessed`, `pr_base`,
     * `default_branch`, or `unknown`.
     *
     * A string rather than an enum for the reason [ChangedFile.status] is one:
     * `base_source_name` in `changes_json.rs` can grow an arm, and a new arm
     * must not fail the decode of a whole change set. Only one value is acted
     * on — see [baseIsGuessed].
     */
    @SerialName("base_source") val baseSource: String = "",
    @SerialName("base_commit") val baseCommit: String = "",
    @SerialName("head_commit") val headCommit: String = "",
    val insertions: Int = 0,
    val deletions: Int = 0,
    val commits: List<ChangeCommit> = emptyList(),
    val files: List<ChangedFile> = emptyList(),
    @SerialName("working_tree") val workingTree: WorkingTree? = null,
) {
    val isDirty: Boolean
        get() {
            val tree = workingTree ?: return false
            return tree.staged.isNotEmpty() || tree.unstaged.isNotEmpty() ||
                tree.untracked.isNotEmpty() || tree.conflicted.isNotEmpty()
        }

    /**
     * Whether nothing knew what this branch is based on, so a local `main` was
     * assumed.
     *
     * Worth surfacing because it is the only base that can be silently wrong: a
     * guessed base produces a diff that looks exactly like a right one. See
     * `BaseSource` in the protocol, and `changes.set_base` for the affordance
     * that exists to answer it.
     */
    val baseIsGuessed: Boolean get() = baseSource == "guessed"

    companion object {
        val EMPTY = ChangeSet()
    }
}

/**
 * One commit on this branch, as the history list and the commit header draw it.
 *
 * ## Everything has a default, and has to
 *
 * A field this decoder required would fail the decode of the ENTIRE change set
 * — every file and every commit — over one absent key on a runner whose daemon
 * predates it. Kotlin's `ignoreUnknownKeys` covers keys this app does not know;
 * defaults are what cover keys the RUNNER does not know, and they are the half
 * that matters when a phone updates faster than the machines it talks to.
 *
 * ## The counts
 *
 * `files_changed`, `insertions` and `deletions` were hardcoded zeroes in
 * `commits_since` (crates/daemon/src/change_set.rs) for as long as that function
 * existed, and `change_set_json` did not project them, so every client worked
 * around it by summing the file list of whichever commit had been SELECTED —
 * which meant a history row could not say what a commit did until it was
 * opened. `--shortstat` on the same `git log` now answers, with
 * `--diff-merges=first-parent` so a merge's row agrees with the file list it
 * opens.
 *
 * **Zero still means unknown**, which is why [counts] is null for it. A runner
 * on a git older than 2.31 rejects `--diff-merges` and the daemon retries
 * without it, leaving merges at the zeroes they always had; and a commit that
 * genuinely changed nothing is rare enough that showing `+0 −0` for it is a
 * worse trade than staying quiet.
 */
@Serializable
data class ChangeCommit(
    val sha: String = "",
    val subject: String = "",
    /**
     * Everything the author wrote after the subject line.
     *
     * It matters more for agent work than for human work. An agent's commit
     * body is usually the closest thing to a written rationale for what it did
     * — why this approach, what it decided against, what it could not finish —
     * and it is the cheapest context available before reading a single line of
     * diff, which on a phone between two sets is the only context there is time
     * for.
     */
    val body: String? = null,
    val author: String = "",
    val timestamp: Long = 0,
    @SerialName("files_changed") val filesChanged: Int? = null,
    val insertions: Int? = null,
    val deletions: Int? = null,
) {
    val short: String get() = sha.take(8)

    /**
     * `+N −M`, when the daemon actually counted them.
     *
     * Null rather than `0 to 0` for the reason the type comment gives: on this
     * wire a zero is indistinguishable from "this runner could not tell you",
     * and a row claiming `+0 −0` about a merge on an old git would be stating a
     * fact nobody established.
     */
    val counts: Pair<Int, Int>?
        get() {
            val plus = insertions ?: 0
            val minus = deletions ?: 0
            return if (plus > 0 || minus > 0) plus to minus else null
        }

    /**
     * The body with its surrounding whitespace gone, or null if there was none.
     *
     * Null rather than an empty string so every caller's null check is the same
     * shape whether the field was absent (an older runner) or present and empty
     * (a commit with only a subject, which is most of them).
     */
    val bodyText: String? get() = body?.trim()?.ifEmpty { null }

    /**
     * The first paragraph worth putting in a list row.
     *
     * The first paragraph rather than the first line, because a body is prose
     * that was wrapped for a terminal and its first line is half a sentence.
     *
     * Trailer-only paragraphs are skipped. Every commit in this repository ends
     * in `Co-Authored-By:` and most agent commits add more of the same, and a
     * row whose one line of preview reads `Co-Authored-By: Claude …` has spent
     * the most valuable line on the screen saying nothing. Skipped, not
     * stripped: the full body is shown in the header, trailers and all, since on
     * the screen that is ABOUT one commit they are part of what it says.
     */
    val bodyPreview: String?
        get() {
            val text = bodyText ?: return null
            for (paragraph in text.split("\n\n")) {
                val lines = paragraph.split("\n").filter { it.isNotEmpty() }
                if (lines.isEmpty()) continue
                if (lines.all { isTrailer(it) }) continue
                return lines.joinToString(" ").trim()
            }
            return null
        }

    /**
     * How long ago this commit was made, in the same shorthand a working
     * agent's row already uses: `12m`, `3h`, `2d`.
     *
     * `nowSeconds` is an ARGUMENT rather than a clock read in here, for the
     * reason [Terminal.displayDuration] spells out at length — a clock read
     * inside a property is an input Compose cannot observe, so the string
     * freezes until something unrelated forces a redraw. Nothing drives this one
     * on a timer and nothing needs to: the history sheet reads the clock once,
     * when it opens, which is the only moment anybody sees these.
     */
    fun age(nowSeconds: Long): String {
        val seconds = nowSeconds - timestamp
        // A commit dated in the future is a clock that disagrees, not a
        // negative age — which happens for real when a runner and a phone are
        // in different places about NTP. "now" is the smallest honest thing to
        // say about it.
        if (seconds < 60) return "now"
        if (seconds < 3_600) return "${seconds / 60}m"
        if (seconds < 86_400) return "${seconds / 3_600}h"
        return "${seconds / 86_400}d"
    }

    companion object {
        /**
         * `Key: value` at the start of a line, which is git's own shape for a
         * trailer. Deliberately loose — this decides what to show first, not
         * what to keep, so a false positive costs a paragraph's place in a
         * preview and never a line of the body.
         */
        internal fun isTrailer(line: String): Boolean {
            val colon = line.indexOf(':')
            if (colon <= 0) return false
            if (colon + 1 >= line.length || line[colon + 1] != ' ') return false
            return line.substring(0, colon).all { it.isLetter() || it == '-' }
        }
    }
}

/**
 * One changed file, in the shape `file_change_json` emits.
 *
 * Shared by `changes.change_set`'s `files`, its `working_tree.changes`, and
 * `changes.commit_files` — one Rust function, so one type here.
 */
@Serializable
data class ChangedFile(
    val path: String = "",
    /**
     * git's verdict, as the wire's own word: `added`, `modified`, `deleted`,
     * `renamed`, `copied`, `type_changed`, `untracked`, `conflicted`.
     *
     * **Kept as a string and parsed rather than decoded as an enum**, which is
     * where this deliberately differs from iOS. `kotlinx.serialization` throws
     * on an enum value it has never heard of, and this type is nested three
     * deep inside a change set — so one new `FileStatus` arm added to the
     * protocol would cost a phone the entire diff rather than costing one row
     * its letter. That is the same trade every default in this file makes, and
     * an enum is the one shape that cannot make it. See [status].
     */
    @SerialName("status") val statusWord: String = "",
    @SerialName("old_path") val oldPath: String? = null,
    val insertions: Int = 0,
    val deletions: Int = 0,
    val binary: Boolean = false,
) {
    val status: ChangedFileStatus get() = ChangedFileStatus.parse(statusWord)

    /** The leaf, which is the part that identifies the file. */
    val name: String get() = path.substringAfterLast('/')

    /**
     * The directory part, shown under the name. A phone cannot fit
     * `crates/daemon/src/review_ops.rs` on one line at a readable size.
     */
    val directory: String get() = path.substringBeforeLast('/', "")

    /**
     * Whether a tool wrote this file rather than a person or an agent.
     *
     * The rule and the whole of its reasoning are in [GeneratedFile], which is
     * a port of AgentKit's — deliberately not decided again here. Two clients
     * deciding it separately was the failure the hoist in `92058f4` prevents:
     * this tab's fold and the Mac's reading order have to agree about what a
     * lockfile is, or the two screens disagree about what a branch changed.
     */
    val isGenerated: Boolean get() = GeneratedFile.isGenerated(path)
}

/**
 * What git says happened to a path.
 *
 * [UNKNOWN] is the arm iOS has no equivalent of, and it is what makes
 * [ChangedFile.statusWord] safe to widen: a status name this build has never
 * seen lands here, the row draws no letter, and the other forty files still
 * arrive. `file_status_name` in `changes_json.rs` maps the protocol's
 * `Unspecified` to `modified` already, so nothing a current daemon sends
 * reaches it.
 */
enum class ChangedFileStatus(val wire: String) {
    ADDED("added"),
    MODIFIED("modified"),
    DELETED("deleted"),
    RENAMED("renamed"),
    COPIED("copied"),
    TYPE_CHANGED("type_changed"),
    UNTRACKED("untracked"),
    CONFLICTED("conflicted"),
    UNKNOWN("");

    /** The letter in the badge beside a row. */
    val mark: String
        get() = when (this) {
            ADDED, UNTRACKED -> "A"
            MODIFIED -> "M"
            DELETED -> "D"
            RENAMED -> "R"
            COPIED -> "C"
            TYPE_CHANGED -> "T"
            CONFLICTED -> "!"
            UNKNOWN -> ""
        }

    /**
     * The word, in sentence case per `cb13d31`.
     *
     * iOS says "Type Changed"; this says "Type changed", which is the one place
     * these two labels differ and the convention that settles it is Android's.
     */
    val label: String
        get() = when (this) {
            ADDED -> "Added"
            MODIFIED -> "Modified"
            DELETED -> "Deleted"
            RENAMED -> "Renamed"
            COPIED -> "Copied"
            TYPE_CHANGED -> "Type changed"
            UNTRACKED -> "Untracked"
            CONFLICTED -> "Conflicted"
            UNKNOWN -> "Changed"
        }

    companion object {
        fun parse(wire: String): ChangedFileStatus =
            entries.firstOrNull { it.wire == wire && it != UNKNOWN } ?: UNKNOWN
    }
}

@Serializable
data class WorkingTree(
    val staged: List<String> = emptyList(),
    val unstaged: List<String> = emptyList(),
    val untracked: List<String> = emptyList(),
    val conflicted: List<String> = emptyList(),
    /**
     * Every dirty path with its own counts, which is what an "uncommitted"
     * total is a sum of.
     *
     * A file that is staged AND modified again appears twice, once per group,
     * because those are two different diffs of it — so counts are summed per
     * path rather than looked up. Untracked files are here too: they are
     * uncommitted work, and a client counting only what git can diff misses the
     * file an agent just wrote.
     *
     * Decoded leniently because a runner can be behind this app. The wire has
     * carried these fields the whole time and nobody filled them until
     * `24f2c1d` — `git status --porcelain=v2` reports status and no line counts
     * — so an older runner sends zeroes, or nothing at all where untracked
     * files are concerned. An app that refused to decode a change set without
     * them would show such a runner nothing at all rather than a number less.
     */
    val changes: List<ChangedFile>? = null,
)

/** The answer to `changes.commit_files`. */
@Serializable
data class CommitFilesReply(val files: List<ChangedFile> = emptyList())

/**
 * The daemon's answer for one file, before it becomes drawable lines.
 *
 * Structured hunks rather than unified text: taking the numbers the daemon
 * already computed beats re-deriving them from `@@` headers, which can silently
 * be off by one. The Mac scraped that text until `c2f1117` gave `changes diff` a
 * `--json` and every client one builder.
 *
 * Hunk boundaries survive into [lines] as a jump in the numbering rather than
 * as nesting, which is what [DiffLayout.hunks] cuts a file back up on. A phone
 * has no room for a `@@` header and the jump already says a gap is there.
 */
@Serializable
data class FileDiffReply(
    val path: String = "",
    /**
     * Why there are no hunks, when the reason is not "nothing changed":
     * `binary`, `submodule`, `combined_diff`, `malformed`.
     *
     * An empty list on its own reads as unchanged, which for a binary file is a
     * lie. See `DiffUnsupported` in the protocol.
     */
    val unsupported: String? = null,
    /**
     * Whether the daemon cut this patch off.
     *
     * **On the wire since `file_diff_json` existed and dropped by iOS**, whose
     * `FileDiff` decodes `hunks` and `unsupported` and nothing else — so a
     * phone reading a very large file today shows the part it was sent and
     * calls it the file. Decoded here a phase early, with no reader yet, on the
     * same terms `Terminal.isChangesPane` was in phase 1: the field costs one
     * line, and the phase that draws the notice should not have to reopen this
     * file to get it. See [truncationNotice].
     */
    val truncated: Boolean = false,
    /**
     * Whether this is a merge shown against its first parent only.
     *
     * Also on the wire and also dropped by iOS. It is the honest caption for
     * the one comparison on this screen that is not the whole truth — see
     * [DiffScope.Commit], which is `{sha}^1..{sha}` and never `git show`.
     */
    @SerialName("firstParentOfMerge") val firstParentOfMerge: Boolean = false,
    val hunks: List<DiffHunkReply> = emptyList(),
) {
    /**
     * Flattened into the same line model the agent transcript's diffs already
     * use, so one row composable draws both.
     */
    fun lines(): List<DiffComputation.Line> {
        val out = ArrayList<DiffComputation.Line>()
        for (hunk in hunks) {
            for (line in hunk.lines) {
                out.add(
                    DiffComputation.Line(
                        id = out.size,
                        kind = when (line.kind) {
                            "added" -> DiffComputation.Kind.ADDED
                            "removed" -> DiffComputation.Kind.REMOVED
                            else -> DiffComputation.Kind.CONTEXT
                        },
                        oldNumber = line.oldNumber,
                        newNumber = line.newNumber,
                        text = line.text,
                    )
                )
            }
        }
        return out
    }

    /** Why this patch is not all of the file, when it is not. Null when it is. */
    val truncationNotice: String?
        get() = if (truncated) "This patch was cut short. It’s too big to send whole." else null

    /**
     * That this comparison is a merge against one side of itself.
     *
     * The honest caption for the one comparison on this screen that is not the
     * whole truth: [DiffScope.Commit] is `{sha}^1..{sha}` and never `git show`,
     * so a merge's patch here is what the merge did to the branch it landed ON
     * and says nothing about the branch it brought in.
     */
    val mergeNotice: String?
        get() = if (firstParentOfMerge) {
            "This is a merge, shown against its first parent only."
        } else {
            null
        }

    /**
     * The sentences that belong ABOVE this patch, in order.
     *
     * Both of these are notices AROUND a patch that is really there, and neither
     * is a reason a file has no lines — an audit of this screen got that
     * backwards once, and the two live together here so the next reader cannot.
     * The reasons a file has no lines are [unsupported], a file git has never
     * seen, and a fetch still in flight; all three are answered elsewhere and
     * none of them is a notice.
     */
    val notices: List<String> get() = listOfNotNull(truncationNotice, mergeNotice)
}

@Serializable
data class DiffHunkReply(
    val index: Int = 0,
    val header: String = "",
    @SerialName("oldStart") val oldStart: Int = 0,
    @SerialName("newStart") val newStart: Int = 0,
    val lines: List<DiffLineReply> = emptyList(),
)

@Serializable
data class DiffLineReply(
    val kind: String = "context",
    @SerialName("oldNumber") val oldNumber: Int? = null,
    @SerialName("newNumber") val newNumber: Int? = null,
    val text: String = "",
    @SerialName("noNewline") val noNewline: Boolean = false,
)

/**
 * Which comparison the diff is showing.
 *
 * A sealed interface with the sha as an associated value, which is iOS's
 * spelling and not the Mac's, and for iOS's reason: `changes.file_diff` takes
 * ONE `scope` string, and `Session::file_diff` in `crates/client/src/session.rs`
 * matches it as `"branch"`, `"local"`, `"staged"`, `"unstaged"` — and
 * `sha => Kind::Commit(sha)` for anything else. By the time a comparison
 * reaches the FFI it is already a single value carrying both facts, and
 * splitting it in two on the way there would manufacture the state the Mac then
 * keeps consistent by hand: a commit scope with no sha beside it, which is a
 * diff of nothing.
 *
 * It also makes switching from one commit to the next a change of scope like
 * any other, so the one reset in `ChangesStore` covers it.
 */
sealed interface DiffScope {
    /**
     * The `scope` argument `changes.file_diff` is given.
     *
     * A sha is its own scope name, which is not a coincidence but the protocol:
     * the arm for anything unrecognized is `sha => Kind::Commit(sha)`. Only a
     * full-length hex sha out of `changes.change_set` can reach it from here, so
     * it cannot collide with the names below.
     */
    val wire: String

    val label: String

    data object Branch : DiffScope {
        override val wire: String get() = "branch"
        override val label: String get() = "Branch"
    }

    data object Local : DiffScope {
        override val wire: String get() = "local"
        override val label: String get() = "Uncommitted"
    }

    /**
     * One commit against its FIRST parent.
     *
     * That is what `Selector::Commit` diffs in `crates/daemon/src/file_diff.rs`
     * — `{sha}^1` against `{sha}`, falling back to the empty tree for a root
     * commit — and deliberately not `git show`, which prints a combined diff
     * for a merge that an ordinary parser reads as nonsense. `changes.commit_files`
     * counts the same comparison, so the file list and the patches under it
     * cannot disagree about what the commit did.
     */
    data class Commit(val sha: String) : DiffScope {
        override val wire: String get() = sha
        override val label: String get() = "Commit"
    }

    /** The sha, when a commit is what is being shown, and null otherwise. */
    val commitSha: String? get() = (this as? Commit)?.sha

    companion object {
        /**
         * The two the comparison control offers, which is not every case.
         *
         * A getter rather than a stored list, and not by taste: a `val` here is
         * initialized when `DiffScope.Companion` is, which happens while
         * `DiffScope` itself is still being loaded, so `Branch` and `Local` are
         * null at that moment and the list holds a null forever. Kotlin issues
         * no warning for it. `ChangesDecodeTest` caught it, which is the one
         * place it would have been caught before a screen drew a blank chip.
         *
         * A Commit segment would be a control that cannot answer its own
         * question: tapping it says nothing about WHICH commit, so it would have
         * to open something, and the thing it would open is the history row
         * directly beneath it.
         */
        val offered: List<DiffScope> get() = listOf(Branch, Local)

        /**
         * Read a [wire] value back, which is the same rule in reverse.
         *
         * `staged` and `unstaged` are scopes the daemon accepts and this app
         * never asks for; they are named here so a bookmark carrying one is not
         * mistaken for a sha and sent as a commit that does not exist.
         */
        fun parse(wire: String): DiffScope = when (wire) {
            "", "branch" -> Branch
            "local", "staged", "unstaged" -> Local
            else -> Commit(wire)
        }
    }
}
