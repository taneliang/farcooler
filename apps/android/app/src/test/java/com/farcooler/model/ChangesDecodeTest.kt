package com.farcooler.model

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The `changes.*` wire, transcribed key for key.
 *
 * The payloads below are copied out of `crates/client/src/changes_json.rs` —
 * `change_set_json`, `file_change_json` and `file_diff_json` — the way
 * `NeedsYouTest` transcribes `Session::changes_inbox` and `FleetDecodeTest`
 * transcribes `Session::fleet`. Every key that function emits is here, including
 * the ones this app deliberately does not read, so a rename on either end fails
 * a test rather than going quiet on a phone as a permanently-zero count.
 *
 * That failure mode is the whole point. `Connection` decodes with
 * `ignoreUnknownKeys = true` and every field in `Changes.kt` has a default —
 * both deliberate, so that a runner older than this app still draws a diff — and
 * the price of that tolerance is that a misspelled `@SerialName` produces a
 * screen full of zeroes and no error anywhere. There is no emulator or device
 * for this phase and no UI yet, so this file is the only thing standing between
 * a typo and a silent wrong answer.
 *
 * Values are non-default throughout: `false` where the default is `false` is a
 * test that passes when the decode does nothing at all.
 */
class ChangesDecodeTest {
    /** The same configuration `Connection` decodes with. */
    private val json = Json { ignoreUnknownKeys = true }

    // ---- change_set_json ----

    /**
     * `change_set_json`, whole. Snake_case throughout, which is where it differs
     * from `Session::fleet` beside it — the fleet builds camelCase keys that
     * happen to match Kotlin property names and this does not.
     */
    private val changeSetPayload = """
        {
          "branch": "feat/review-on-a-phone",
          "base_ref": "origin/main",
          "base_source": "guessed",
          "base_commit": "1111111111111111111111111111111111111111",
          "head_commit": "2222222222222222222222222222222222222222",
          "insertions": 402,
          "deletions": 118,
          "commits": [
            {
              "sha": "aaaaaaaabbbbbbbbccccccccddddddddeeeeeeee",
              "subject": "feat(android): read a diff on a phone",
              "body": "Why this shape and not the other.\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
              "author": "E-Liang Tan",
              "timestamp": 1755900000,
              "files_changed": 7,
              "insertions": 300,
              "deletions": 90
            }
          ],
          "files": [
            {
              "path": "crates/daemon/src/review_ops.rs",
              "status": "modified",
              "old_path": null,
              "insertions": 82,
              "deletions": 13,
              "binary": false
            },
            {
              "path": "apps/android/app/src/main/res/raw/logo.png",
              "status": "added",
              "old_path": "apps/android/logo.png",
              "insertions": 0,
              "deletions": 0,
              "binary": true
            }
          ],
          "working_tree": {
            "staged": ["crates/core/src/feed.rs"],
            "unstaged": ["crates/core/src/feed.rs", "README.md"],
            "untracked": ["notes/scratch.md"],
            "conflicted": ["crates/daemon/src/rpc.rs"],
            "changes": [
              {
                "path": "crates/core/src/feed.rs",
                "status": "modified",
                "old_path": null,
                "insertions": 4,
                "deletions": 1,
                "binary": false
              },
              {
                "path": "crates/core/src/feed.rs",
                "status": "modified",
                "old_path": null,
                "insertions": 9,
                "deletions": 2,
                "binary": false
              },
              {
                "path": "README.md",
                "status": "modified",
                "old_path": null,
                "insertions": 3,
                "deletions": 0,
                "binary": false
              },
              {
                "path": "notes/scratch.md",
                "status": "untracked",
                "old_path": null,
                "insertions": 12,
                "deletions": 0,
                "binary": false
              }
            ]
          }
        }
    """.trimIndent()

    private fun changeSet() = json.decodeFromString(ChangeSet.serializer(), changeSetPayload)

    @Test
    fun `every key the change set sends is decoded under the name this app reads it by`() {
        val set = changeSet()
        assertEquals("feat/review-on-a-phone", set.branch)
        assertEquals("origin/main", set.baseRef)
        assertEquals("guessed", set.baseSource)
        assertEquals("1111111111111111111111111111111111111111", set.baseCommit)
        assertEquals("2222222222222222222222222222222222222222", set.headCommit)
        assertEquals(402, set.insertions)
        assertEquals(118, set.deletions)
        assertEquals(1, set.commits.size)
        assertEquals(2, set.files.size)
        // The four groups plus the per-path counts, which are a different list
        // from all four of them.
        val tree = requireNotNull(set.workingTree)
        assertEquals(listOf("crates/core/src/feed.rs"), tree.staged)
        assertEquals(listOf("crates/core/src/feed.rs", "README.md"), tree.unstaged)
        assertEquals(listOf("notes/scratch.md"), tree.untracked)
        assertEquals(listOf("crates/daemon/src/rpc.rs"), tree.conflicted)
        assertEquals(4, tree.changes?.size)
        assertTrue(set.isDirty)
    }

    @Test
    fun `a guessed base is the one base worth warning about`() {
        assertTrue(changeSet().baseIsGuessed)
        // Every other arm `base_source_name` can produce is a base somebody or
        // something actually recorded.
        for (source in listOf("recorded", "upstream", "pr_base", "default_branch", "unknown")) {
            val set = json.decodeFromString(
                ChangeSet.serializer(), """{"base_source":"$source"}"""
            )
            assertFalse(source, set.baseIsGuessed)
        }
    }

    @Test
    fun `a commit carries its body and its own three counts`() {
        val commit = changeSet().commits[0]
        assertEquals("aaaaaaaabbbbbbbbccccccccddddddddeeeeeeee", commit.sha)
        assertEquals("aaaaaaaa", commit.short)
        assertEquals("feat(android): read a diff on a phone", commit.subject)
        assertEquals("E-Liang Tan", commit.author)
        assertEquals(1_755_900_000L, commit.timestamp)
        assertEquals(7, commit.filesChanged)
        assertEquals(300 to 90, commit.counts)
        // The first paragraph, not the first line, and not the trailer.
        assertEquals("Why this shape and not the other.", commit.bodyPreview)
    }

    /**
     * A commit the daemon could not count — every merge on a git older than
     * 2.31, since `--diff-merges` is rejected and the retry drops the counts.
     * Zero must read as "nobody said", not as `+0 −0`.
     */
    @Test
    fun `zero counts are unknown rather than nothing`() {
        val absent = json.decodeFromString(ChangeCommit.serializer(), """{"sha":"a","subject":"s"}""")
        assertNull(absent.counts)
        val zeroed = json.decodeFromString(
            ChangeCommit.serializer(), """{"sha":"a","insertions":0,"deletions":0}"""
        )
        assertNull(zeroed.counts)
    }

    @Test
    fun `a body of only trailers previews nothing`() {
        val commit = ChangeCommit(
            sha = "a",
            body = "Co-Authored-By: Claude <x@y>\nClaude-Session: https://example.invalid",
        )
        assertNull(commit.bodyPreview)
        // Skipped in the preview, never stripped from the body itself.
        assertTrue(requireNotNull(commit.bodyText).contains("Co-Authored-By"))
    }

    @Test
    fun `a file change decodes every key including the ones that are usually null`() {
        val files = changeSet().files
        assertEquals("crates/daemon/src/review_ops.rs", files[0].path)
        assertEquals(ChangedFileStatus.MODIFIED, files[0].status)
        assertNull(files[0].oldPath)
        assertEquals(82, files[0].insertions)
        assertEquals(13, files[0].deletions)
        assertFalse(files[0].binary)
        assertEquals("review_ops.rs", files[0].name)
        assertEquals("crates/daemon/src", files[0].directory)

        assertEquals(ChangedFileStatus.ADDED, files[1].status)
        assertEquals("apps/android/logo.png", files[1].oldPath)
        assertTrue(files[1].binary)
    }

    /**
     * Every word `file_status_name` can emit, and one it cannot.
     *
     * The last case is the reason `statusWord` is a string: a status name this
     * build has never heard of must cost one row its letter, and an enum on the
     * wire would cost the whole change set.
     */
    @Test
    fun `every status the wire can send is understood and an unknown one is survivable`() {
        val expected = mapOf(
            "added" to ChangedFileStatus.ADDED,
            "modified" to ChangedFileStatus.MODIFIED,
            "deleted" to ChangedFileStatus.DELETED,
            "renamed" to ChangedFileStatus.RENAMED,
            "copied" to ChangedFileStatus.COPIED,
            "type_changed" to ChangedFileStatus.TYPE_CHANGED,
            "untracked" to ChangedFileStatus.UNTRACKED,
            "conflicted" to ChangedFileStatus.CONFLICTED,
        )
        for ((wire, status) in expected) {
            val file = json.decodeFromString(ChangedFile.serializer(), """{"path":"p","status":"$wire"}""")
            assertEquals(wire, status, file.status)
        }
        val future = json.decodeFromString(
            ChangedFile.serializer(), """{"path":"p","status":"unmerged_both_added"}"""
        )
        assertEquals(ChangedFileStatus.UNKNOWN, future.status)
        assertEquals("p", future.path)
    }

    /** An older runner that sends nothing but what it has. */
    @Test
    fun `a change set with only a branch decodes to a clean empty one`() {
        val set = json.decodeFromString(ChangeSet.serializer(), """{"branch":"main"}""")
        assertEquals("main", set.branch)
        assertTrue(set.commits.isEmpty())
        assertTrue(set.files.isEmpty())
        assertNull(set.workingTree)
        assertFalse(set.isDirty)
        assertFalse(set.baseIsGuessed)
    }

    @Test
    fun `a key this app has never heard of does not cost the change set`() {
        val set = json.decodeFromString(
            ChangeSet.serializer(),
            """{"branch":"main","stack_position":3,"files":[{"path":"a","reviewed_by":"nobody"}]}""",
        )
        assertEquals("main", set.branch)
        assertEquals("a", set.files[0].path)
    }

    // ---- commit_files ----

    /**
     * `Session::commit_files` wraps the same `file_change_json` in one key.
     *
     * Believed rather than second-guessed, because the host determines it:
     * `commit_files` in `crates/daemon/src/file_diff.rs` runs
     * `git diff --name-status -z --find-renames` alongside the `--numstat -z`
     * pass over the SAME left-hand ref, so `added` and `renamed` on this wire are
     * git's verdicts. That was not always so — it used to run `--numstat` alone,
     * which cannot say whether a path was created, and wrote `modified` into
     * everything it did not recognize as a rename.
     */
    @Test
    fun `a commit's file list decodes through the same file shape`() {
        val reply = json.decodeFromString(
            CommitFilesReply.serializer(),
            """
            {
              "files": [
                {
                  "path": "Cargo.lock",
                  "status": "modified",
                  "old_path": null,
                  "insertions": 3011,
                  "deletions": 1002,
                  "binary": false
                }
              ]
            }
            """.trimIndent(),
        )
        assertEquals(1, reply.files.size)
        assertEquals("Cargo.lock", reply.files[0].path)
        assertEquals(3011, reply.files[0].insertions)
        assertTrue(reply.files[0].isGenerated)
    }

    // ---- file_diff_json ----

    /**
     * `file_diff_json`, whole — and camelCase, which is the trap.
     *
     * One FFI, two casings: the change set above is `base_ref` and `old_path`,
     * and this is `oldStart`, `newNumber`, `noNewline`, `firstParentOfMerge`. A
     * `@SerialName` missing from either half is a field stuck at its default
     * forever, and on this one that means every line of every patch drawn with no
     * line number beside it.
     */
    private val fileDiffPayload = """
        {
          "path": "crates/daemon/src/review_ops.rs",
          "unsupported": null,
          "truncated": true,
          "firstParentOfMerge": true,
          "hunks": [
            {
              "index": 0,
              "header": "@@ -10,3 +10,4 @@ fn inbox()",
              "oldStart": 10,
              "newStart": 10,
              "lines": [
                {"kind": "context", "oldNumber": 10, "newNumber": 10, "text": "fn inbox() {", "noNewline": false},
                {"kind": "removed", "oldNumber": 11, "newNumber": null, "text": "    old()", "noNewline": false},
                {"kind": "added", "oldNumber": null, "newNumber": 11, "text": "    new()", "noNewline": false},
                {"kind": "added", "oldNumber": null, "newNumber": 12, "text": "    more()", "noNewline": true}
              ]
            }
          ]
        }
    """.trimIndent()

    @Test
    fun `every key one file's patch sends is decoded under the name this app reads it by`() {
        val diff = json.decodeFromString(FileDiffReply.serializer(), fileDiffPayload)
        assertEquals("crates/daemon/src/review_ops.rs", diff.path)
        assertNull(diff.unsupported)
        assertTrue("truncated must survive", diff.truncated)
        assertTrue("firstParentOfMerge is camelCase on this half", diff.firstParentOfMerge)
        assertEquals(1, diff.hunks.size)
        val hunk = diff.hunks[0]
        assertEquals(0, hunk.index)
        assertEquals("@@ -10,3 +10,4 @@ fn inbox()", hunk.header)
        assertEquals(10, hunk.oldStart)
        assertEquals(10, hunk.newStart)
        assertEquals(4, hunk.lines.size)
        assertTrue(hunk.lines[3].noNewline)
        assertEquals("This patch was cut short. It’s too big to send whole.", diff.truncationNotice)
    }

    @Test
    fun `hunks flatten into the line model the transcript's diffs already use`() {
        val lines = json.decodeFromString(FileDiffReply.serializer(), fileDiffPayload).lines()
        assertEquals(4, lines.size)
        assertEquals(listOf(0, 1, 2, 3), lines.map { it.id })
        assertEquals(
            listOf(
                DiffComputation.Kind.CONTEXT,
                DiffComputation.Kind.REMOVED,
                DiffComputation.Kind.ADDED,
                DiffComputation.Kind.ADDED,
            ),
            lines.map { it.kind },
        )
        assertEquals(listOf(10, 11, null, null), lines.map { it.oldNumber })
        assertEquals(listOf(10, null, 11, 12), lines.map { it.newNumber })
        assertEquals("    new()", lines[2].text)
    }

    /**
     * Every reason `file_diff_json` gives for a patch with no hunks in it. An
     * empty list on its own reads as unchanged, which for a binary file is a lie.
     */
    @Test
    fun `each unsupported reason has words of its own`() {
        for (code in listOf("binary", "submodule", "combined_diff", "malformed")) {
            val diff = json.decodeFromString(
                FileDiffReply.serializer(), """{"path":"p","unsupported":"$code"}"""
            )
            assertEquals(code, diff.unsupported)
            assertTrue(diff.hunks.isEmpty())
        }
    }

    @Test
    fun `a kind the wire has never sent reads as context rather than failing`() {
        val diff = json.decodeFromString(
            FileDiffReply.serializer(),
            """{"hunks":[{"lines":[{"kind":"moved","text":"x"}]}]}""",
        )
        assertEquals(DiffComputation.Kind.CONTEXT, diff.lines()[0].kind)
    }

    // ---- branch.list, which the base picker reads ----

    /**
     * `Session::branches`, transcribed key for key — and TYPE for type.
     *
     * `crates/client/src/session.rs`, and camelCase where the change set is
     * snake_case — the same two-casings-out-of-one-FFI trap `Changes.kt` records
     * at the top, which is exactly the sort of thing a `@SerialName` gets wrong
     * once and then reports as a permanently-false flag.
     *
     * **`remote` is a remote NAME or null, never a boolean**, and that is the
     * reason this test was rewritten rather than left alone. It used to send
     * `"remote": true`, which is not a shape the runner can produce:
     * `git::BranchInfo.remote` is an `Option<String>`. So it asserted the app's
     * own mistake back at itself and stayed green while the base picker failed
     * to decode a single real repository. A transcription is only worth having
     * if it is copied from the producer.
     *
     * `updatedAt` is MILLISECONDS here. The protocol carries seconds and that
     * line multiplies by a thousand, so a client reading it as seconds would put
     * every branch fifty-six thousand years in the past.
     *
     * Values are non-default throughout, including `local: false` on the remote
     * row, which is what makes `whereItLives` a real answer rather than a
     * default that happened to be right.
     */
    @Test
    fun `a branch list decodes every key session dot rs emits`() {
        val payload = """
            {
              "branches": [
                {
                  "name": "main",
                  "local": true,
                  "remote": "origin",
                  "checkedOut": true,
                  "subject": "fix: green means one thing again, on both phones",
                  "updatedAt": 1755900000000
                },
                {
                  "name": "origin/feat/review",
                  "local": false,
                  "remote": "origin",
                  "checkedOut": false,
                  "subject": "",
                  "updatedAt": null
                },
                {
                  "name": "wip/local-only",
                  "local": true,
                  "remote": null,
                  "checkedOut": false,
                  "subject": "scratch",
                  "updatedAt": 1755899000000
                }
              ]
            }
        """.trimIndent()

        val reply = json.decodeFromString(BranchListReply.serializer(), payload)
        assertEquals(3, reply.branches.size)

        val main = reply.branches[0]
        assertEquals("main", main.name)
        assertTrue(main.local)
        assertEquals("origin", main.remote)
        assertTrue(main.checkedOut)
        assertEquals("fix: green means one thing again, on both phones", main.subject)
        assertEquals(1_755_900_000_000, main.updatedAt)
        assertEquals("local and remote", main.whereItLives)

        val remote = reply.branches[1]
        assertFalse(remote.local)
        assertFalse(remote.checkedOut)
        assertEquals("remote", remote.whereItLives)
        // Absent rather than zero, so a row can decline to say rather than lie.
        assertNull(remote.updatedAt)

        // An explicit null is the other half of what an `Option<String>` emits,
        // and it is the half that used to throw: kotlinx will not put a null
        // into a non-nullable property, default or no default.
        val local = reply.branches[2]
        assertNull(local.remote)
        assertEquals("local", local.whereItLives)
    }

    /**
     * The Mac's `BranchInfo.age` thresholds, off a millisecond clock.
     *
     * The `/ 1000` is the whole difference between this and the Mac's line, and
     * omitting it is the mistake that dates every branch to 1970 — so the first
     * case here is a real timestamp against a real clock rather than a
     * hand-picked difference, which is the only arrangement that catches it.
     */
    @Test
    fun `a branch age reads milliseconds and stops at the same thresholds`() {
        val now = 1_755_900_000_000
        fun at(ms: Long) = BranchRef(name = "b", updatedAt = ms).age(now)

        assertEquals("", BranchRef(name = "b").age(now))
        // Zero is git having no committer date, not the epoch.
        assertEquals("", at(0))
        // Read as seconds this would be "20323d". The unit is the test.
        assertEquals("2h", at(now - 7_200_000))
        assertEquals("now", at(now))
        // A tip dated in the future is a clock disagreement, not a negative age.
        assertEquals("now", at(now + 60_000))
        // Under a minute still rounds up to 1m rather than collapsing to "0m".
        assertEquals("1m", at(now - 30_000))
        assertEquals("59m", at(now - 3_599_000))
        assertEquals("1h", at(now - 3_600_000))
        assertEquals("23h", at(now - 86_399_000))
        assertEquals("1d", at(now - 86_400_000))
        assertEquals("30d", at(now - 30L * 86_400_000))
    }

    /** A runner too old to send a key leaves it at its default, not at an error. */
    @Test
    fun `a branch with only a name still decodes`() {
        val reply = json.decodeFromString(
            BranchListReply.serializer(), """{"branches":[{"name":"main"}]}"""
        )
        assertEquals("main", reply.branches.single().name)
        // Nothing was sent about where it lives, so nothing is claimed — and
        // `whereItLives` falls to "local", which is what an unqualified branch
        // name means.
        assertEquals("local", reply.branches.single().whereItLives)
        assertEquals(BranchListReply(), json.decodeFromString(BranchListReply.serializer(), "{}"))
    }

    // ---- the scope, which is a wire value too ----

    /**
     * The names `Session::file_diff` matches, and the rule for everything else.
     *
     * `"" | "branch" => Range`, `"local"`, `"staged"`, `"unstaged"`, and
     * `sha => Kind::Commit(sha)`. The last arm is why a sha is its own scope
     * name, and why nothing here may invent a fourth word.
     */
    @Test
    fun `a scope is one wire string and a sha is its own scope name`() {
        assertEquals("branch", DiffScope.Branch.wire)
        assertEquals("local", DiffScope.Local.wire)
        assertEquals("deadbeef", DiffScope.Commit("deadbeef").wire)
        assertEquals(DiffScope.Branch, DiffScope.parse("branch"))
        assertEquals(DiffScope.Branch, DiffScope.parse(""))
        assertEquals(DiffScope.Local, DiffScope.parse("local"))
        assertEquals(DiffScope.Commit("deadbeef"), DiffScope.parse("deadbeef"))
        assertEquals(listOf(DiffScope.Branch, DiffScope.Local), DiffScope.offered)
        assertEquals("deadbeef", DiffScope.Commit("deadbeef").commitSha)
        assertNull(DiffScope.Branch.commitSha)
    }
}
