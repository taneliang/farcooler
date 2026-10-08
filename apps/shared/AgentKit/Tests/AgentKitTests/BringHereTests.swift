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
            place: { _ in steps.append("place") },
            clear: { _ in steps.append("clear"); return .success(true) })
        XCTAssertEqual(steps, [])
        guard case .said(let words)? = issue else { return XCTFail("\(String(describing: issue))") }
        XCTAssertTrue(words.contains("pasted block"), words)
    }

    /// A clear that didn't go leaves the text in both places: placed in the
    /// composer, and said to be in the box still.
    func testAFailedClearKeepsTheTextInTheComposer() async {
        for failure: AgentConversation.SendFailure in [.refused(what: "changed"), .refused(what: "partly"), .timedOut] {
            var placed: String?
            let issue = await BringHere.run(
                read: { .success("from the box") }, place: { placed = $0 }, clear: { _ in .failure(failure) })
            XCTAssertEqual(placed, "from the box", "\(failure)")
            guard case .draftLeftInTerminal(let words)? = issue else { return XCTFail("\(failure)") }
            XCTAssertTrue(words.hasPrefix("The draft is here"), words)
        }
    }

    /// An empty box: nothing to bring, nothing cleared.
    func testAnEmptyBoxBringsNothing() async {
        var steps: [String] = []
        let issue = await BringHere.run(
            read: { .success("") }, place: { _ in steps.append("place") },
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
