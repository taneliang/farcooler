import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The orchestrator owns the task list (ov-184). The Mac reads tasks, answers
/// an agent's question, and asks the orchestrator; it files, edits, re-statuses
/// and deletes nothing. The CLI and daemon keep every task command, because
/// agents work the list through them.
struct TaskManagementRemovedTests {
    /// `…/apps/macos/Sources`, from this file's path.
    private static var sources: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { root.deleteLastPathComponent() }  // CeremonyTests, Tests, macos
        return root.appendingPathComponent("Sources")
    }

    /// The task commands that write the list, as the CLI spells them in an
    /// argv (`"task", "create"`) and as the daemon names them on the wire
    /// (`"task.create"`).
    private static let writes = "create|set|update|move|block|set_status"

    /// Each task write a source in `texts` names, as "text in file".
    static func taskWrites(in texts: [(String, String)]) -> [String] {
        let argv = try! NSRegularExpression(pattern: "\"task\"\\s*,\\s*\"(\(writes))\"")
        let wire = try! NSRegularExpression(pattern: "\"task\\.(\(writes))\"")
        return texts.flatMap { name, text in
            [argv, wire].flatMap { regex in
                let range = NSRange(text.startIndex..., in: text)
                return regex.matches(in: text, range: range).compactMap { match in
                    Range(match.range, in: text).map { "\(text[$0]) in \(name)" }
                }
            }
        }
    }

    private static func macSources() throws -> [(String, String)] {
        let walker = try #require(FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil))
        return try walker.compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" }.map {
            ($0.lastPathComponent, try String(contentsOf: $0, encoding: .utf8))
        }
    }

    @Test("No Mac view or client names task create, set, update, move or block")
    func noMacSourceWritesATask() throws {
        let texts = try Self.macSources()
        #expect(texts.count > 100, "found \(texts.count) sources under \(Self.sources.path), so this proves nothing")
        // The scan reads the same spelling the client uses for what it keeps.
        #expect(texts.contains { $0.1.contains("\"task\", \"note\"") }, "the answer's argv moved")
        #expect(texts.contains { $0.1.contains("\"task\", \"show\"") }, "the read's argv moved")
        let named = Self.taskWrites(in: texts)
        #expect(named.isEmpty, "a task write the orchestrator owns: \(named)")
    }

    @Test("A planted task write is found, in either spelling")
    func aPlantedWriteIsFound() {
        let texts = [
            ("A.swift", "run([\"task\", \"create\", \"--title\", t])"),
            ("B.swift", "run([\"task\",\n \"set\", key])"),
            ("C.swift", "let m = \"task.set_status\""),
            ("D.swift", "run([\"task\", \"note\", key, \"--kind\", \"answer\"])"),
        ]
        #expect(
            Self.taskWrites(in: texts)
                == ["\"task\", \"create\" in A.swift", "\"task\",\n \"set\" in B.swift", "\"task.set_status\" in C.swift"])
    }
}

/// Ask the Orchestrator: a reference to the task in the orchestrator's
/// composer, and never an orchestrator started for it.
@MainActor
struct AskOrchestratorTests {
    private static let workspace = "0198f2c0-0000-7000-8000-0000000000dd"
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"

    private static let row = TaskRow(
        id: "t-9", key: "bil-9", title: "Invoice PDF export", status: .inProgress, statusSince: .now)

    private static func pane(_ id: String, state: String = "running", chat: Bool = true) -> BoardPane {
        var terminal = Terminal(id: id, short: id, title: id, preset: "claude", state: state, epoch: 0)
        terminal.paneMode = chat ? "agent" : "terminal"
        terminal.role = "orchestrator"
        terminal.workspace = workspace
        let worktree = Worktree(
            id: "w", short: "w", task: "w", branch: "main", repository: "shop", host: "", path: "/tmp/w",
            state: "active", terminals: [terminal], repositoryID: repo, workspace: workspace)
        return BoardPane(terminal: terminal, worktree: worktree)
    }

    @Test("The draft names the task and ends where the person types")
    func theDraftNamesTheTask() {
        #expect(AskOrchestrator.draft(for: Self.row) == "About bil-9 (“Invoice PDF export”): ")
    }

    @Test("Asking leaves the draft in the orchestrator's composer, joined to what waits there")
    func askingDraftsIntoTheComposer() {
        let handoff = ComposerHandoff()
        let orchestrator = Self.pane("orch")
        #expect(AskOrchestrator.ask(about: Self.row, of: orchestrator, handoff: handoff))
        #expect(handoff.waiting["orch"] == "About bil-9 (“Invoice PDF export”): ")
        // Nothing a person left there is overwritten.
        #expect(AskOrchestrator.ask(about: Self.row, of: orchestrator, handoff: handoff))
        #expect(handoff.waiting["orch"] == "About bil-9 (“Invoice PDF export”): \n\nAbout bil-9 (“Invoice PDF export”): ")
    }

    @Test("With no orchestrator, nothing is drafted anywhere")
    func noOrchestratorDraftsNothing() {
        let handoff = ComposerHandoff()
        #expect(!AskOrchestrator.ask(about: Self.row, of: nil, handoff: handoff))
        #expect(handoff.waiting.isEmpty)
    }

    @Test("The workspace's orchestrator is found only while one is running")
    func theOrchestratorIsFoundOnlyWhileRunning() {
        func fleet(_ state: String?) -> Fleet {
            Fleet(
                runtimeHealthy: true, livePanes: 0,
                worktrees: state.map { [Self.pane("orch", state: $0).worktree] } ?? [], branchPrefix: nil)
        }
        let summary = WorkspaceSummary(
            id: Self.workspace, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: Self.repo)
        #expect(WorkspaceScreen.orchestrator(of: summary, host: "", in: fleet("running")) != nil)
        #expect(WorkspaceScreen.orchestrator(of: summary, host: "", in: fleet("exited")) == nil)
        #expect(WorkspaceScreen.orchestrator(of: summary, host: "", in: fleet(nil)) == nil)
        // Main can't have one.
        #expect(WorkspaceScreen.orchestrator(of: .implicit(repository: Self.repo), host: "", in: fleet("running")) == nil)
    }

    @Test("The disabled item says what to do, and the sentence is the one the card names")
    func theDisabledSentence() {
        #expect(AskOrchestrator.unavailable == "Start an orchestrator to ask about this task")
        #expect(!AskOrchestrator.Action.unavailable.available)
    }

    // MARK: A terminal orchestrator: pasted by the daemon, or copied

    /// What a delivery did: the paste it asked for, and what it copied.
    private final class Spy {
        var pasted: [String] = []
        var copied: [String] = []
    }

    private func deliver(
        _ pane: BoardPane, pasteSucceeds: Bool, spy: Spy, handoff: ComposerHandoff = ComposerHandoff()
    ) async -> AskOrchestrator.Delivery {
        await AskOrchestrator.deliver(
            Self.row, to: pane,
            paste: { text in
                spy.pasted.append(text)
                return pasteSucceeds
            },
            copy: { spy.copied.append($0) }, handoff: handoff)
    }

    @Test("A terminal orchestrator is asked of the daemon to paste, and nothing is copied")
    func aTerminalOrchestratorIsPastedByTheDaemon() async {
        let spy = Spy()
        let handoff = ComposerHandoff()
        let got = await deliver(Self.pane("orch", chat: false), pasteSucceeds: true, spy: spy, handoff: handoff)
        #expect(got == .pasted)
        #expect(spy.pasted == ["About bil-9 (“Invoice PDF export”): "])
        #expect(spy.copied.isEmpty)
        #expect(handoff.waiting.isEmpty, "a terminal has no composer to fill")
    }

    @Test("A paste the daemon refuses is copied instead, with the notice")
    func aRefusedPasteIsCopied() async {
        let spy = Spy()
        let got = await deliver(Self.pane("orch", chat: false), pasteSucceeds: false, spy: spy)
        #expect(got == .copied)
        #expect(spy.copied == ["About bil-9 (“Invoice PDF export”):"])
        #expect(AskOrchestrator.copiedNotice(for: Self.row) == "Copied a reference to bil-9. Paste it into the orchestrator.")
    }

    @Test("A chat orchestrator takes it in its composer and the daemon is never asked")
    func aChatOrchestratorIsNotPasted() async {
        let spy = Spy()
        let handoff = ComposerHandoff()
        let got = await deliver(Self.pane("orch"), pasteSucceeds: true, spy: spy, handoff: handoff)
        #expect(got == .composer)
        #expect(spy.pasted.isEmpty && spy.copied.isEmpty)
        #expect(handoff.waiting["orch"] == "About bil-9 (“Invoice PDF export”): ")
    }

    @Test("What Ask the Orchestrator sends to the daemon or the clipboard never ends in a line break")
    func neverAnEnter() async {
        let spy = Spy()
        var row = Self.row
        row.title = "Line\nbreak\r and\n"
        for ok in [true, false] {
            _ = await AskOrchestrator.deliver(
                row, to: Self.pane("orch", chat: false),
                paste: { spy.pasted.append($0); return ok }, copy: { spy.copied.append($0) },
                handoff: ComposerHandoff())
        }
        for text in spy.pasted + spy.copied { #expect(!text.contains(where: \.isNewline)) }
        for text in spy.pasted + spy.copied { #expect(!text.hasSuffix("\n") && !text.hasSuffix("\r"), "\(text.debugDescription)") }
        #expect(!spy.pasted.isEmpty && !spy.copied.isEmpty)
    }
}
