import Foundation
import Testing

@testable import Far_Cooler

/// Reading `farcooler changes files <workspace> <sha>` back into rows.
///
/// This has teeth for the reason the rest of this target does: it is a rule
/// that is invisible by looking. The CLI prints a FIXED-WIDTH table rather than
/// JSON — `println!("+{:<5} -{:<5} {}")` in `crates/cli/src/changes.rs` — so the
/// separator between a count and a path is a run of spaces, and so is the
/// separator inside `My Documents/notes.md`. Splitting on whitespace reads the
/// first case correctly and silently truncates the second, which is a file that
/// appears in a commit's list under half its name and opens a diff for a path
/// that does not exist.
///
/// The format string is the contract. If it ever changes, these are what say so
/// rather than a file list that quietly comes back empty.
struct CommitFilesTests {
    @Test func aFileIsItsTwoCountsAndAllOfTheRest() {
        let rows = ChangesStore.parseCommitFiles("+12    -3     crates/core/src/feed.rs\n")
        #expect(rows.count == 1)
        #expect(rows.first?.path == "crates/core/src/feed.rs")
        #expect(rows.first?.insertions == 12)
        #expect(rows.first?.deletions == 3)
    }

    /// The case that decides how this is written at all.
    @Test func aPathWithSpacesSurvivesWhole() {
        let rows = ChangesStore.parseCommitFiles("+1     -0     docs/My Notes/read me.md\n")
        #expect(rows.first?.path == "docs/My Notes/read me.md")
    }

    /// A count wider than its padding runs into the next field, so the padding
    /// cannot be treated as a fixed offset.
    @Test func aCountWiderThanItsPaddingStillParses() {
        let rows = ChangesStore.parseCommitFiles("+123456 -654321 Cargo.lock\n")
        #expect(rows.first?.path == "Cargo.lock")
        #expect(rows.first?.insertions == 123_456)
        #expect(rows.first?.deletions == 654_321)
    }

    @Test func everyLineBecomesARow() {
        let rows = ChangesStore.parseCommitFiles(
            """
            +12    -3     a.swift
            +0     -40    b.swift
            +7     -7     c/d.swift
            """)
        #expect(rows.count == 3)
        #expect(rows.map(\.path) == ["a.swift", "b.swift", "c/d.swift"])
    }

    /// Nothing committed, or nothing this commit touched. Empty is an answer
    /// and must not read as a parse that fell over.
    @Test func nothingIsNoRowsRatherThanOneEmptyOne() {
        #expect(ChangesStore.parseCommitFiles("").isEmpty)
        #expect(ChangesStore.parseCommitFiles("\n\n").isEmpty)
    }

    /// A line that is not this shape is skipped rather than guessed at. A row
    /// with no path is the one that matters: it would open a diff for `""`.
    @Test func aLineThatIsNotAFileIsSkipped() {
        #expect(ChangesStore.parseCommitFiles("fatal: bad object\n").isEmpty)
        #expect(ChangesStore.parseCommitFiles("+12    -3     \n").isEmpty)
        #expect(ChangesStore.parseCommitFiles("12 3 a.swift\n").isEmpty)
    }

    /// The status is nil on purpose, and it is worth pinning: the daemon builds
    /// this list from `--numstat`, which counts lines and never says whether a
    /// file was added or deleted. Defaulting to `Modified` would put that word
    /// beside a file the commit created.
    @Test func aFileFromACommitClaimsNoStatus() {
        let rows = ChangesStore.parseCommitFiles("+9     -0     new/file.swift\n")
        #expect(rows.first?.status == nil)
    }
}
