import Foundation
import Testing

@testable import Far_Cooler

/// What a commit says about itself, and what happens when a runner cannot say
/// it.
///
/// The reason this is in the target with teeth is one line of Swift that is not
/// written here: `ChangeCommit` decodes INSIDE `ChangeSet`, and the synthesized
/// `Decodable` throws on a missing key. So a non-optional `body` would mean a
/// runner whose `farcooler` predates that field failing the decode of the
/// ENTIRE change set — every file and every commit, gone, over one absent key —
/// and the pane drawing a worktree with no changes in it. Nothing about that
/// failure names the field that caused it, and nothing about it is visible
/// until you meet an old runner.
struct ChangeCommitDecodeTests {
    private func decode(_ json: String) throws -> ChangeSet {
        try JSONDecoder().decode(ChangeSet.self, from: Data(json.utf8))
    }

    /// A change set from a runner that predates every field added past
    /// `timestamp`. The assertion that matters is the FILE list surviving: this
    /// is not a test about a commit's counts, it is a test about one missing
    /// key not taking the diff with it.
    @Test func anOlderRunnerStillHandsOverAWholeChangeSet() throws {
        let set = try decode(
            """
            {"branch":"feature","base_ref":"main","base_commit":"abc","head_commit":"def",
             "insertions":12,"deletions":3,
             "commits":[{"sha":"aaaa1111","subject":"Teach the pane to walk",
                         "author":"Ada","timestamp":1750000000}],
             "files":[{"path":"a.swift","status":"modified","old_path":null,
                       "insertions":12,"deletions":3,"binary":false}],
             "working_tree":null}
            """)
        #expect(set.files.count == 1)
        #expect(set.commits.count == 1)
        #expect(set.commits.first?.body == nil)
        #expect(set.commits.first?.counts == nil)
        #expect(set.commits.first?.filesChanged == nil)
    }

    /// A current runner, which sends all four.
    @Test func aCurrentRunnerCarriesTheBodyAndTheCounts() throws {
        let set = try decode(
            """
            {"branch":"feature","base_ref":"main","base_commit":"abc","head_commit":"def",
             "insertions":12,"deletions":3,
             "commits":[{"sha":"aaaa1111","subject":"Teach the pane to walk",
                         "body":"Because a branch is a sequence.",
                         "author":"Ada","timestamp":1750000000,
                         "files_changed":4,"insertions":12,"deletions":3}],
             "files":[],"working_tree":null}
            """)
        let c = try #require(set.commits.first)
        #expect(c.bodyText == "Because a branch is a sequence.")
        #expect(c.filesChanged == 4)
        #expect(c.counts?.insertions == 12)
        #expect(c.counts?.deletions == 3)
    }

    /// The `body` field exists in the proto and defaults to an empty string, so
    /// most commits arrive carrying one that says nothing. Nil and empty have
    /// to read the same to every caller, or half the rows in the history would
    /// reserve a line for a paragraph that is not there.
    @Test func anEmptyBodyIsTheSameAsNoBody() throws {
        let set = try decode(
            """
            {"branch":"f","base_ref":"main","base_commit":"a","head_commit":"b",
             "insertions":0,"deletions":0,
             "commits":[{"sha":"aaaa1111","subject":"Fix a typo","body":"   \\n  ",
                         "author":"Ada","timestamp":1750000000}],
             "files":[],"working_tree":null}
            """)
        #expect(set.commits.first?.bodyText == nil)
        #expect(set.commits.first?.bodyPreview == nil)
    }
}

/// Zero on this wire means "this runner could not tell you", not "nothing
/// changed".
///
/// The case is real and not rare: a runner on a git older than 2.31 rejects
/// `--diff-merges`, the daemon retries the `git log` without it, and every
/// merge on the branch comes back with the zeroes it always had. A row reading
/// `+0 −0` about a merge that brought in four hundred lines is not a smaller
/// claim than the truth — it is a different one.
struct ChangeCommitCountsTests {
    private func commit(insertions: Int?, deletions: Int?) throws -> ChangeCommit {
        var fields = #""sha":"a1","subject":"s","author":"a","timestamp":0"#
        if let insertions { fields += #","insertions":\#(insertions)"# }
        if let deletions { fields += #","deletions":\#(deletions)"# }
        return try JSONDecoder().decode(ChangeCommit.self, from: Data("{\(fields)}".utf8))
    }

    @Test func twoZeroesAreNoNumberAtAll() throws {
        #expect(try commit(insertions: 0, deletions: 0).counts == nil)
    }

    @Test func aMissingPairIsAlsoNoNumber() throws {
        #expect(try commit(insertions: nil, deletions: nil).counts == nil)
    }

    /// One side at zero is a real answer — a commit that only added lines, or
    /// only removed them — and must not be swallowed with the other case.
    @Test func oneSideAtZeroIsStillAnAnswer() throws {
        let added = try commit(insertions: 9, deletions: 0)
        #expect(added.counts?.insertions == 9)
        #expect(added.counts?.deletions == 0)
        let removed = try commit(insertions: 0, deletions: 40)
        #expect(removed.counts?.insertions == 0)
        #expect(removed.counts?.deletions == 40)
    }
}

/// The one line of a commit body a history row has room for.
///
/// The rule with teeth is the trailer skip. Every commit in this repository
/// ends in `Co-Authored-By:` and most agent commits add more of the same, so a
/// preview that took the first paragraph literally would spend the most
/// valuable line in the popover saying `Co-Authored-By: Claude …` — on every
/// row, identically, which is a list that has stopped distinguishing anything.
struct ChangeCommitBodyPreviewTests {
    private func preview(_ body: String) throws -> String? {
        let escaped =
            body
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\n", with: "\\n")
        return try JSONDecoder().decode(
            ChangeCommit.self,
            from: Data(
                #"{"sha":"a1","subject":"s","author":"a","timestamp":0,"body":"\#(escaped)"}"#.utf8)
        ).bodyPreview
    }

    /// The first PARAGRAPH, not the first line. A body is prose wrapped for an
    /// eighty-column terminal, so its first line is half a sentence — and half
    /// a sentence is worse than none, because it reads as a truncation bug.
    @Test func aWrappedParagraphIsRejoinedIntoOneLine() throws {
        #expect(
            try preview("The pane kept the scroll in the view,\nwhich the layout tore down.")
                == "The pane kept the scroll in the view, which the layout tore down.")
    }

    @Test func onlyTheFirstParagraphIsPreviewed() throws {
        #expect(try preview("What it does.\n\nWhy it does it.") == "What it does.")
    }

    /// The case this rule exists for.
    @Test func aTrailerOnlyParagraphIsSkipped() throws {
        #expect(
            try preview("Co-Authored-By: Claude <noreply@anthropic.com>\n\nThe real reason.")
                == "The real reason.")
    }

    /// Trailers at the END are the ordinary shape, and the paragraph before
    /// them is what anybody wants to read.
    @Test func trailersAfterTheProseChangeNothing() throws {
        #expect(
            try preview("The real reason.\n\nCo-Authored-By: Claude <noreply@anthropic.com>")
                == "The real reason.")
    }

    /// Nothing but trailers is nothing worth a line, and an empty string here
    /// would draw a blank row where a paragraph should be.
    @Test func abodyThatIsOnlyTrailersPreviewsNothing() throws {
        #expect(
            try preview(
                "Co-Authored-By: Claude <noreply@anthropic.com>\nClaude-Session: https://example")
                == nil)
    }

    /// A paragraph where only SOME lines look like trailers is prose. The rule
    /// is deliberately loose about what a trailer is, so it must be strict
    /// about when it applies: a false positive is allowed to cost a paragraph
    /// its place, never a line of the body.
    @Test func aParagraphIsProseIfAnyLineIsProse() throws {
        #expect(
            try preview("Fixes: the base was guessed\nand nothing said so.")
                == "Fixes: the base was guessed and nothing said so.")
    }

    /// `Key:value` with no space, and a bare colon at the start of a line, are
    /// not git trailers. Reading them as trailers would drop real prose.
    @Test func aColonAloneDoesNotMakeATrailer() throws {
        #expect(try preview("note:no space here") == "note:no space here")
        #expect(try preview(": leading colon") == ": leading colon")
        #expect(try preview("Trailing colon:") == "Trailing colon:")
    }
}
