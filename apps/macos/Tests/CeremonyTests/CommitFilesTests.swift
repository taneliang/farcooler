import Foundation
import Testing

@testable import Far_Cooler

/// Reading `farcooler changes files <workspace> <sha> --json` back into rows.
///
/// The status letter on a commit's file rows is what these are really about.
/// The daemon has known it since `changes.commit_files` gained its
/// `git diff --name-status --find-renames` pass, but this app reaches that call
/// through the CLI, and for as long as `changes files` printed only a
/// fixed-width `+ins -del path` table the letter died in the pipe and every
/// file in a commit was badged with a dot. `--json` is what carries it, and
/// `status` surviving the decode is the contract worth pinning.
struct CommitFilesTests {
    private func decode(_ json: String) throws -> CommitFiles {
        try JSONDecoder().decode(CommitFiles.self, from: Data(json.utf8))
    }

    @Test func aFileIsItsStatusItsTwoCountsAndItsPath() throws {
        let files = try decode(
            """
            {"files":[{"path":"crates/core/src/feed.rs","insertions":12,"deletions":3,
            "binary":false,"status":"modified","old_path":null}]}
            """
        ).files
        #expect(files.count == 1)
        #expect(files.first?.path == "crates/core/src/feed.rs")
        #expect(files.first?.insertions == 12)
        #expect(files.first?.deletions == 3)
        #expect(files.first?.status == .modified)
    }

    /// The whole point of the change: a file the commit CREATED says so, rather
    /// than saying "Modified" or saying nothing.
    @Test func everyStatusGitGivesSurvivesTheTrip() throws {
        let files = try decode(
            """
            {"files":[
            {"path":"a.swift","insertions":9,"deletions":0,"binary":false,"status":"added","old_path":null},
            {"path":"b.swift","insertions":0,"deletions":40,"binary":false,"status":"deleted","old_path":null},
            {"path":"c.swift","insertions":1,"deletions":1,"binary":false,"status":"modified","old_path":null},
            {"path":"d.swift","insertions":0,"deletions":0,"binary":false,"status":"copied","old_path":"c.swift"},
            {"path":"e.sh","insertions":0,"deletions":0,"binary":false,"status":"type_changed","old_path":null}
            ]}
            """
        ).files
        #expect(files.map(\.status) == [.added, .deleted, .modified, .copied, .typeChanged])
    }

    /// A rename carries where the file came from, and the pane draws it. Losing
    /// `old_path` would leave a row saying `R` with nothing to say `R` from.
    @Test func aRenameKnowsWhatItWasCalled() throws {
        let files = try decode(
            """
            {"files":[{"path":"crates/cli/src/changes.rs","insertions":4,"deletions":4,
            "binary":false,"status":"renamed","old_path":"crates/cli/src/diff.rs"}]}
            """
        ).files
        #expect(files.first?.status == .renamed)
        #expect(files.first?.oldPath == "crates/cli/src/diff.rs")
    }

    /// The case that used to decide how the text parser was written, kept
    /// because it is still the file most likely to come back wrong.
    @Test func aPathWithSpacesSurvivesWhole() throws {
        let files = try decode(
            """
            {"files":[{"path":"docs/My Notes/read me.md","insertions":1,"deletions":0,
            "binary":false,"status":"added","old_path":null}]}
            """
        ).files
        #expect(files.first?.path == "docs/My Notes/read me.md")
    }

    /// Nothing committed, or nothing this commit touched. Empty is an answer
    /// and must not read as a decode that fell over — `readCommitFiles` tells
    /// the two apart, and a warning triangle is the wrong thing to draw here.
    @Test func nothingIsNoRowsRatherThanAFailure() throws {
        #expect(try decode(#"{"files":[]}"#).files.isEmpty)
    }

    /// A binary file has no counts worth showing and the pane badges it `B`,
    /// which it can only do if the flag arrives.
    @Test func aBinaryFileSaysSo() throws {
        let files = try decode(
            """
            {"files":[{"path":"art/icon.png","insertions":0,"deletions":0,
            "binary":true,"status":"added","old_path":null}]}
            """
        ).files
        #expect(files.first?.binary == true)
    }
}

/// The fixed-width table, which `readCommitFiles` still reads when a runner's
/// `farcooler` predates `--json` on this command.
///
/// This has teeth for the reason the rest of this target does: it is a rule
/// that is invisible by looking. Fields are padded rather than delimited —
/// `println!("+{:<5} -{:<5} {}")` in `crates/cli/src/changes.rs` — so the
/// separator between a count and a path is a run of spaces, and so is the
/// separator inside `My Documents/notes.md`. Splitting on whitespace reads the
/// first case correctly and silently truncates the second, which is a file that
/// appears in a commit's list under half its name and opens a diff for a path
/// that does not exist.
struct CommitFilesTableFallbackTests {
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

    /// A row from the table claims no status, because the table has none to
    /// give. `M` would put "Modified" beside a file the commit created — the
    /// badge draws a dot instead, and it is now the only thing that ever does.
    @Test func aFileFromTheTableClaimsNoStatus() {
        let rows = ChangesStore.parseCommitFiles("+9     -0     new/file.swift\n")
        #expect(rows.first?.status == nil)
    }
}
