import XCTest

@testable import AgentKit

/// Bring Here (ov-369, R-28): the box's draft read, put in the composer
/// ahead of what it held, then the box cleared of exactly that; the text
/// never in neither place.
final class BringHereTests: XCTestCase {
    private func build(_ capabilities: [String]) -> DaemonBuild {
        DaemonBuild(version: "t", matches: true, platform: "t", capabilities: Set(capabilities))
    }

    func testOfferedForClaudeOnARunnerThatServesIt() {
        XCTAssertTrue(BringHere.offered(preset: "claude", build: build(["bring_draft"])))
        XCTAssertTrue(BringHere.offered(preset: "claude:opus", build: build(["bring_draft"])))
        XCTAssertFalse(BringHere.offered(preset: "claude", build: build(["compose"])), "an older runner")
        XCTAssertFalse(BringHere.offered(preset: "codex", build: build(["bring_draft"])), "codex isn't served")
        XCTAssertFalse(BringHere.offered(preset: "claude", build: nil))
    }

    func testTheBoxComesFirstThenTheComposersText() {
        XCTAssertEqual(BringHere.merged(box: "fix the login\nthen the tests", native: "and the docs"),
                       "fix the login\nthen the tests\nand the docs")
        XCTAssertEqual(BringHere.merged(box: "from the box\n", native: ""), "from the box")
        XCTAssertEqual(BringHere.merged(box: "from the box", native: "  \n"), "from the box")
        XCTAssertEqual(BringHere.merged(box: "", native: "mine"), "mine")
    }

    /// Read, placed, then cleared of exactly the text read, in that order.
    func testAReadThenAClearOfExactlyThat() async {
        var steps: [String] = []
        var composer = "and the docs"
        let issue = await BringHere.run(
            read: { steps.append("read"); return .success("fix the login") },
            place: { text in steps.append("place"); composer = BringHere.merged(box: text, native: composer) },
            withdraw: { _ in steps.append("withdraw") },
            clear: { text in steps.append("clear \(text)"); return .success(true) })
        XCTAssertNil(issue)
        XCTAssertEqual(steps, ["read", "place", "clear fix the login"], "in the composer before the box is touched")
        XCTAssertEqual(composer, "fix the login\nand the docs")
    }

    /// A refused read moves nothing, and no clear is asked for.
    func testARefusedReadMovesNothing() async {
        var steps: [String] = []
        let issue = await BringHere.run(
            read: { .failure(.refused(what: "pasted")) },
            place: { _ in steps.append("place") }, withdraw: { _ in steps.append("withdraw") },
            clear: { _ in steps.append("clear"); return .success(true) })
        XCTAssertEqual(steps, [])
        guard case .said(let words)? = issue else { return XCTFail("\(String(describing: issue))") }
        XCTAssertTrue(words.contains("pasted block"), words)
    }

    /// A clear that went `partly`, or never answered, may have taken some of
    /// the box's text: the composer keeps its copy and says it's in both.
    func testAClearThatMayHaveTakenTextKeepsItInTheComposer() async {
        for failure: AgentConversation.SendFailure in [.refused(what: "partly"), .timedOut, .lost(notSent: false)] {
            var placed: String?
            var withdrawn = false
            let issue = await BringHere.run(
                read: { .success("from the box") }, place: { placed = $0 }, withdraw: { _ in withdrawn = true },
                clear: { _ in .failure(failure) })
            XCTAssertEqual(placed, "from the box", "\(failure)")
            XCTAssertFalse(withdrawn, "\(failure)")
            guard case .draftLeftInTerminal(let words)? = issue else { return XCTFail("\(failure)") }
            XCTAssertTrue(words.hasPrefix("The draft is here"), words)
        }
    }

    /// A clear refused with the box whole leaves the text in the box alone:
    /// the composer gives it back, and never says "Clear it there", which
    /// would have the person delete text the composer lacks.
    func testARefusedClearGivesTheTextBack() async {
        for what in ["changed", "too_tall", "typing", "sending", "unfamiliar", "prompt"] {
            var composer = "mine"
            var withdrawn: String?
            let issue = await BringHere.run(
                read: { .success("from the box") },
                place: { composer = BringHere.merged(box: $0, native: composer) },
                withdraw: { withdrawn = $0; composer = BringHere.withdrawn(box: $0, from: composer) },
                clear: { _ in .failure(.refused(what: what)) })
            XCTAssertEqual(withdrawn, "from the box", what)
            XCTAssertEqual(composer, "mine", what)
            if case .draftLeftInTerminal(let words)? = issue { XCTFail("\(what): \(words)") }
            XCTAssertNotNil(issue, what)
        }
        var gone: String?
        let notConnected = await BringHere.run(
            read: { .success("x") }, place: { _ in }, withdraw: { gone = $0 }, clear: { _ in .failure(.lost(notSent: true)) })
        XCTAssertEqual(gone, "x", "a clear that never left")
        guard case .said? = notConnected else { return XCTFail("\(String(describing: notConnected))") }
    }

    /// A clear that answers `cleared: false` found the box empty: sent from
    /// the terminal or taken by another device. The composer's copy would send
    /// it twice.
    func testAClearThatClearedNothingGivesTheTextBack() async {
        var composer = ""
        let issue = await BringHere.run(
            read: { .success("fix the login") },
            place: { composer = BringHere.merged(box: $0, native: composer) },
            withdraw: { composer = BringHere.withdrawn(box: $0, from: composer) },
            clear: { _ in .success(false) })
        XCTAssertEqual(composer, "")
        guard case .said(let words)? = issue else { return XCTFail("\(String(describing: issue))") }
        XCTAssertTrue(words.contains("emptied"), words)
    }

    /// Taking the box's text back out leaves what the composer held, and
    /// what was typed after.
    func testWithdrawnTakesOnlyTheBrought() {
        let merged = BringHere.merged(box: "fix the login\nthen the tests", native: "and the docs")
        XCTAssertEqual(BringHere.withdrawn(box: "fix the login\nthen the tests", from: merged), "and the docs")
        XCTAssertEqual(BringHere.withdrawn(box: "fix the login", from: "fix the login"), "")
        XCTAssertEqual(BringHere.withdrawn(box: "fix the login", from: "I rewrote it"), "I rewrote it", "left alone")
        XCTAssertEqual(BringHere.merged(box: "    indented\nmore\n", native: ""), "    indented\nmore", "the indent is the draft's")
    }

    /// An empty box: nothing to bring, nothing cleared.
    func testAnEmptyBoxBringsNothing() async {
        var steps: [String] = []
        let issue = await BringHere.run(
            read: { .success("") }, place: { _ in steps.append("place") }, withdraw: { _ in steps.append("withdraw") },
            clear: { _ in steps.append("clear"); return .success(false) })
        XCTAssertEqual(steps, [])
        XCTAssertNil(issue)
    }

    /// Every refusal word the runner names has its own words or a handoff.
    func testEveryRefusalHasWords() {
        for word in ["pasted", "too_tall", "cursor", "typing", "sending", "unsupported", "not_running"] {
            guard case .said(let words) = BringHere.issue(for: .refused(what: word)) else { return XCTFail(word) }
            XCTAssertFalse(words.contains("_"), words)
        }
        XCTAssertEqual(BringHere.issue(for: .refused(what: "prompt")), .handoff)
    }
}
