import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Bring Here in the Mac's native view (ov-369, R-28): a send refused for
/// the draft in claude's box offers Bring Here beside Show Terminal where the
/// runner serves it; Bring Here reads the box, puts its text ahead of the
/// composer's, then clears the box of exactly that; a clear that fails leaves
/// the text in both and says so. The runner's own answer is the daemon's
/// `bring_tests`.
@MainActor
@Suite(.serialized)
struct NativeBringHereTests {
    /// A runner that records each call and answers as told.
    actor StandInDrafts: DraftSink {
        var calls: [String] = []
        var box = "fix the login\nthen the tests"
        var read: RunnerCore.Failure?
        var clear: RunnerCore.Failure?
        /// The clear finds the box already empty.
        var emptied = false

        func set(read: RunnerCore.Failure? = nil, clear: RunnerCore.Failure? = nil, emptied: Bool = false) {
            self.read = read
            self.clear = clear
            self.emptied = emptied
        }

        func bringDraft(terminal: String, expected: String?) async throws -> (text: String, cleared: Bool) {
            guard let expected else {
                calls.append("read \(terminal)")
                if let read { throw read }
                return (box, false)
            }
            calls.append("clear \(expected)")
            if let clear { throw clear }
            if emptied { return ("", false) }
            box = ""
            return (expected, true)
        }
    }

    static let refused = { (what: String) in RunnerCore.Failure.refused("no", word: "resource-conflict", what: what) }

    static func model(_ drafts: StandInDrafts?, program: String = "claude") throws -> NativePaneModel {
        let model = NativeAgentTests.model(try NativeAgentTests.terminal(program: program))
        model.program = program
        // Every runner with `bring_draft` has `compose`: line breaks kept.
        model.rich = true
        model.drafts = drafts
        return model
    }

    @Test("A draft in the box offers Bring Here beside Show Terminal, where it's served")
    func offeredWhereServed() async throws {
        let model = try Self.model(StandInDrafts())
        let seen = NativeAgentTests.Seen()
        let window = NativeAgentTests.window(NativeAgentTests.Probe(seen: seen, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        defer { window.close() }
        model.issue = NativePaneModel.issue(for: Self.refused("draft"))
        #expect(model.issue == .draftInTerminal)
        await NativeAgentTests.settle(window)
        #expect(seen.ids.contains("native-bring-here"))

        model.drafts = nil
        await NativeAgentTests.settle(window)
        #expect(!seen.ids.contains("native-bring-here"), "a runner without bring_draft: Show Terminal alone")
        #expect(seen.ids.contains("native-send-issue"))

        let codex = try Self.model(StandInDrafts(), program: "codex")
        #expect(!codex.offersBringHere, "codex's box isn't read")
    }

    @Test("Bring Here puts the box's text ahead of the composer's, then clears exactly that")
    func bringsThenClears() async throws {
        let drafts = StandInDrafts()
        let model = try Self.model(drafts)
        model.draft = "and the docs"
        model.issue = .draftInTerminal
        await model.bringHere()
        #expect(model.draft == "fix the login\nthen the tests\nand the docs")
        #expect(model.issue == nil)
        #expect(await drafts.calls == ["read \(model.terminal)", "clear fix the login\nthen the tests"])
        #expect(!model.bringing)
    }

    @Test("A refused read moves nothing and says why")
    func aRefusedReadMovesNothing() async throws {
        let drafts = StandInDrafts()
        await drafts.set(read: Self.refused("too_tall"))
        let model = try Self.model(drafts)
        model.draft = "mine"
        await model.bringHere()
        #expect(model.draft == "mine")
        #expect(await drafts.calls.count == 1, "no clear asked for")
        guard case .said(let words)? = model.issue else { Issue.record("\(String(describing: model.issue))"); return }
        #expect(words.contains("too long to bring here"))
    }

    @Test("A clear refused with the box whole gives the text back, and never says to clear the box")
    func aRefusedClearGivesTheTextBack() async throws {
        let drafts = StandInDrafts()
        await drafts.set(clear: Self.refused("changed"))
        let model = try Self.model(drafts)
        model.draft = "and the docs"
        await model.bringHere()
        #expect(model.draft == "and the docs", "the box holds the only copy")
        #expect(model.issue == .draftInTerminal, "Bring Here and Show Terminal again, no \"clear it there\"")

        await drafts.set(clear: Self.refused("typing"))
        await model.bringHere()
        #expect(model.draft == "and the docs")
        guard case .said(let words)? = model.issue else { Issue.record("\(String(describing: model.issue))"); return }
        #expect(words.contains("so its draft stayed there"))
    }

    @Test("A clear that answers cleared: false gives the text back, so a send can't repeat it")
    func aClearThatClearedNothingGivesTheTextBack() async throws {
        let drafts = StandInDrafts()
        await drafts.set(emptied: true)
        let model = try Self.model(drafts)
        model.draft = "mine"
        await model.bringHere()
        #expect(model.draft == "mine")
        guard case .said(let words)? = model.issue else { Issue.record("\(String(describing: model.issue))"); return }
        #expect(words.contains("emptied"))
    }

    @Test("A clear that went partly or never answered keeps the text here and says it's in the box too")
    func aPartlyClearKeepsTheText() async throws {
        let drafts = StandInDrafts()
        await drafts.set(clear: Self.refused("partly"))
        let model = try Self.model(drafts)
        await model.bringHere()
        #expect(model.draft == "fix the login\nthen the tests")
        guard case .draftLeftInTerminal(let words)? = model.issue else { Issue.record("\(String(describing: model.issue))"); return }
        #expect(words.hasPrefix("The draft is here"))

        await drafts.set(clear: .timedOut("late"))
        model.draft = ""
        await model.bringHere()
        #expect(model.draft == "fix the login\nthen the tests", "a late clear may have gone: the text stays")
        guard case .draftLeftInTerminal(let late)? = model.issue else { Issue.record("late"); return }
        #expect(late.contains("didn’t answer in time"))
    }

    @Test("Without the runner's bring_draft, Bring Here does nothing")
    func nothingWithoutTheRunner() async throws {
        let model = try Self.model(nil)
        model.draft = "mine"
        model.issue = .draftInTerminal
        await model.bringHere()
        #expect(model.draft == "mine" && model.issue == .draftInTerminal)
    }
}
