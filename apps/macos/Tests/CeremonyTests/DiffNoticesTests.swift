import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The two facts a diff carries that are not lines of diff, and that this app
/// could not see until `c2f1117` printed them.
///
/// Both are in the target with teeth for the same reason: neither is visible by
/// looking at the pane. A guessed base draws a diff that looks exactly like a
/// right one, and a submodule draws the same empty file body a genuinely
/// unchanged file does. The only place either becomes checkable is where the
/// bytes turn into a decision, which is here.

/// Where the base came from.
///
/// `base_source` has been on the wire since `BaseSource` existed and reached the
/// phones the whole time; the CLI's copy of the JSON builder never printed it,
/// so this app has never seen it. The decision it feeds is one line —
/// `baseIsGuessed` — and getting it wrong in either direction is a real cost: a
/// false positive puts a permanent warning over correct diffs, and a false
/// negative is the silence that made this worth adding.
struct BaseSourceTests {
    private func decode(_ json: String) throws -> ChangeSet {
        try JSONDecoder().decode(ChangeSet.self, from: Data(json.utf8))
    }

    private func set(baseSource: String?) throws -> ChangeSet {
        let key = baseSource.map { "\"base_source\":\"\($0)\"," } ?? ""
        return try decode(
            """
            {"branch":"feature","base_ref":"main",\(key)
             "base_commit":"abc","head_commit":"def","insertions":12,"deletions":3,
             "commits":[],"files":[],"working_tree":null}
            """)
    }

    /// The one value that means nobody knew.
    @Test func onlyAGuessIsAGuess() throws {
        #expect(try set(baseSource: "guessed").baseIsGuessed)
    }

    /// Every other name `base_source_name` can print. All five are recorded
    /// facts about the repository, and a warning over any of them would be this
    /// pane crying wolf on the diffs it gets right — which is most of them, and
    /// which is what would teach a reader to stop seeing the orange line at all.
    @Test(arguments: ["recorded", "upstream", "pr_base", "default_branch", "unknown"])
    func everythingElseIsAFact(_ source: String) throws {
        #expect(try !set(baseSource: source).baseIsGuessed)
    }

    /// A runner whose `farcooler` predates the key.
    ///
    /// The assertion that matters is not `baseIsGuessed` — it is that the change
    /// set decoded AT ALL. `baseSource` decodes inside `ChangeSet`, so a
    /// non-optional would have failed the whole thing over one absent key, and
    /// the pane would draw a worktree with no changes in it. This is not
    /// hypothetical: the runner this was written against answers exactly this
    /// way.
    @Test func anOlderRunnerSaysNothingAndKeepsItsChangeSet() throws {
        let set = try decode(
            """
            {"branch":"feature","base_ref":"main","base_commit":"abc","head_commit":"def",
             "insertions":12,"deletions":3,"commits":[],
             "files":[{"path":"a.swift","status":"modified","old_path":null,
                       "insertions":12,"deletions":3,"binary":false}],
             "working_tree":null}
            """)
        #expect(set.files.count == 1)
        #expect(set.baseSource == nil)
        // Not a guess, because nobody said it was one. The alternative — warning
        // whenever the field is missing — would put the orange line over every
        // diff on every runner installed before `c2f1117`.
        #expect(!set.baseIsGuessed)
    }
}

/// One file's patch, decoded from what `changes diff --json` prints.
///
/// The bytes below are `farcooler_client::changes_json::file_diff_json`'s, which
/// is the same function the phones read over the FFI — so what is pinned here is
/// one shape for two clients, not this app's private idea of one. The Rust side
/// pins the emitter in `crates/cli/src/changes.rs`; this pins the decode.
struct FileDiffDecodeTests {
    private func decode(_ json: String) throws -> FileDiff {
        try JSONDecoder().decode(FileDiff.self, from: Data(json.utf8))
    }

    /// The bug in miniature: a submodule and an unchanged file both have no
    /// hunks, and until now this app could not tell them apart, so it said
    /// "No textual changes" about a submodule.
    @Test func aSubmoduleIsNotAnUnchangedFile() throws {
        let d = try decode(
            """
            {"path":"vendor/thing","unsupported":"submodule","truncated":false,
             "firstParentOfMerge":false,"hunks":[]}
            """)
        #expect(d.lines.isEmpty)
        #expect(d.unsupportedNote == "Submodule")

        let plain = try decode(
            """
            {"path":"a.swift","unsupported":null,"truncated":false,
             "firstParentOfMerge":false,"hunks":[]}
            """)
        #expect(plain.lines.isEmpty)
        // Nil is what lets the pane keep saying "No textual changes" here, which
        // for this one file is true.
        #expect(plain.unsupportedNote == nil)
        #expect(!plain.truncated)
        #expect(!plain.firstParentOfMerge)
    }

    /// Every reason the daemon can give, and one it cannot.
    ///
    /// The unknown code is the reason `unsupported` is a `String` and not an
    /// enum: a future `DiffUnsupported` variant must cost this app one
    /// unrecognized sentence, not the whole patch.
    @Test func eachReasonSaysSomethingDifferent() throws {
        func note(_ code: String) throws -> String? {
            try decode("""
                {"path":"a","unsupported":"\(code)","truncated":false,
                 "firstParentOfMerge":false,"hunks":[]}
                """).unsupportedNote
        }
        #expect(try note("binary") == FileDiff.binaryNote)
        #expect(try note("submodule") == "Submodule")
        #expect(try note("combined_diff") == FileDiff.mergeNote)
        #expect(try note("malformed") == "This patch could not be read")
        #expect(try note("something_from_2027") == "This patch could not be read")
        // Four sentences, four meanings. A pane that reuses one of these for
        // another is back where it started.
        let all = try Set([note("binary"), note("submodule"), note("combined_diff"), note("malformed")])
        #expect(all.count == 4)
    }

    /// A merge refused and a merge rendered say the same sentence on purpose,
    /// from one constant — see `FileDiff.mergeNote`. The difference between them
    /// is whether there are lines under it, and that is what the pane draws.
    @Test func aMergeSaysTheSameThingWhicheverWayItArrives() throws {
        let refused = try decode(
            """
            {"path":"a","unsupported":"combined_diff","truncated":true,
             "firstParentOfMerge":true,"hunks":[]}
            """)
        #expect(refused.unsupportedNote == FileDiff.mergeNote)

        let rendered = try decode(
            """
            {"path":"a","unsupported":null,"truncated":false,"firstParentOfMerge":true,
             "hunks":[{"index":0,"header":"@@ -1,1 +1,1 @@","oldStart":1,"newStart":1,
              "lines":[{"kind":"added","oldNumber":null,"newNumber":1,"text":"x","noNewline":false}]}]}
            """)
        #expect(rendered.unsupportedNote == nil)
        #expect(rendered.firstParentOfMerge)
        #expect(rendered.lines.count == 1)
    }

    /// Hunks flatten into one list and their boundaries survive as the jump
    /// between two line numbers — which is what `ChangesPane.body(of:lines:)`
    /// finds its gaps from, and the reason it never wanted `@@` headers.
    ///
    /// The bytes are a trimmed copy of what the live daemon actually answered.
    @Test func hunksBecomeOneListWithTheGapsStillFindable() throws {
        let d = try decode(
            """
            {"firstParentOfMerge":false,"path":"Sources/VerdelaCore/Keymap.swift",
             "truncated":false,"unsupported":null,
             "hunks":[
              {"header":"@@ -88,3 +88,3 @@","index":0,"oldStart":88,"newStart":88,"lines":[
               {"kind":"context","newNumber":88,"noNewline":false,"oldNumber":88,"text":"  cc(\\"0\\"),"},
               {"kind":"removed","newNumber":null,"noNewline":false,"oldNumber":89,"text":"  .nextIntent,"},
               {"kind":"added","newNumber":89,"noNewline":false,"oldNumber":null,"text":"  .nextGroup,"}]},
              {"header":"@@ -140,1 +140,1 @@","index":1,"oldStart":140,"newStart":140,"lines":[
               {"kind":"context","newNumber":140,"noNewline":false,"oldNumber":140,"text":"}"}]}]}
            """)

        #expect(d.lines.count == 4)
        #expect(d.lines.map(\.id) == [0, 1, 2, 3])
        #expect(d.lines.map(\.kind).map(String.init(describing:))
            == ["context", "removed", "added", "context"])
        // A removed line has no new-side number, which is what stops the pane
        // reading it as the start of a gap.
        #expect(d.lines[1].newNumber == nil)
        #expect(d.lines[1].oldNumber == 89)
        #expect(d.lines[2].oldNumber == nil)
        #expect(d.lines[2].newNumber == 89)
        // The gap between the two hunks: 89 to 140 with nothing in between.
        #expect(d.lines[2].newNumber == 89)
        #expect(d.lines[3].newNumber == 140)
        #expect(d.lines[3].text == "}")
    }

    /// The two notices that are not refusals. Both leave real lines on screen
    /// and change what those lines mean, which is why losing them was a
    /// confident half-truth rather than a blank.
    @Test func truncationAndAFirstParentMergeSurviveTheDecode() throws {
        let d = try decode(
            """
            {"path":"src/main.rs","unsupported":null,"truncated":true,"firstParentOfMerge":true,
             "hunks":[{"index":0,"header":"@@ -1,2 +1,3 @@","oldStart":1,"newStart":1,
              "lines":[{"kind":"added","oldNumber":null,"newNumber":2,"text":"let x = 1;",
                        "noNewline":false}]}]}
            """)
        #expect(d.truncated)
        #expect(d.firstParentOfMerge)
        #expect(d.lines.count == 1)
        #expect(d.lines[0].text == "let x = 1;")
    }

    /// How an older runner is detected, and the whole of it.
    ///
    /// `--json` is global on `farcooler`, so a runner whose CLI predates
    /// `c2f1117` accepts the flag on this command and prints the human patch
    /// regardless. Nothing in that reply says "I am old" — the only signal is
    /// that it is not this shape. `hunks` is the required key that makes the
    /// decode itself the probe, so `changesDiff` can hand the same bytes to
    /// `parseUnified` and draw a diff rather than a warning triangle.
    @Test func anAnswerWithoutHunksIsNotThisShape() throws {
        // A human patch, which is what an older runner actually sends.
        #expect(throws: (any Error).self) {
            try decode("@@ -1,2 +1,2 @@\n context\n-old\n+new\n")
        }
        // Well-formed JSON that is nonetheless some other object.
        #expect(throws: (any Error).self) {
            try decode("""
                {"path":"a.swift","unsupported":null,"truncated":false,"firstParentOfMerge":false}
                """)
        }
        // Nothing at all, which is what a failed subprocess leaves behind.
        #expect(throws: (any Error).self) { try decode("") }
    }

    /// The fallback still parses what it always parsed.
    ///
    /// Kept, not deleted, for the reason `parseCommitFiles` was kept when
    /// `changes files` gained `--json`: a runner one version behind must still
    /// draw its diff. The three notices are what it cannot recover, and that is
    /// the bounded cost of being behind.
    @Test @MainActor func theOlderRunnersOutputStillBecomesLines() {
        let lines = DaemonClient.parseUnified(
            """
            @@ -12,4 +12,4 @@ func step() {
             let a = 1
            -let b = 2
            +let b = 3
             let c = 4
            """)
        #expect(lines.count == 4)
        #expect(lines.map(\.text) == ["let a = 1", "let b = 2", "let b = 3", "let c = 4"])
        #expect(lines[0].oldNumber == 12)
        #expect(lines[0].newNumber == 12)
        #expect(lines[1].newNumber == nil)
        #expect(lines[2].oldNumber == nil)
        // Numbering carries on past the removed and added pair rather than
        // counting both against the same side.
        #expect(lines[3].oldNumber == 14)
        #expect(lines[3].newNumber == 14)
    }
}
