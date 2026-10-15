import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The first run's Reading and Charter screens (ov-519 phase 2b): when they
/// open in place of a start, what they write before starting, and what they
/// draw.
@MainActor
@Suite(.serialized)
struct CharterSetupTests {
    /// A survey as `repo survey --json` prints it, with GitHub's rules, a
    /// CLAUDE.md and Codex as the first agent found.
    static func survey(text: String = "Ci.yml runs cargo test.", harness: String = "codex") -> Data {
        Data(
            """
            {"answers":[
             {"key":"workflow","value":"pull_requests","text":"PR: each change is a pull request.","source":"main requires pull requests"},
             {"key":"done_means","value":"","text":"\(text)","source":".github/workflows/ci.yml"},
             {"key":"review","value":"","text":"I review each pull request.","source":"default"},
             {"key":"lanes","value":"2","text":"Up to 2 agents at once.","source":"default"},
             {"key":"autonomy","value":"","text":"Agents may commit and push their own branches.","source":"default"},
             {"key":"initiative","value":"on","text":"Initiative: on.","source":"default"},
             {"key":"instructions","value":"","text":"Agents follow CLAUDE.md, which they already read.","source":"CLAUDE.md (32 lines)"},
             {"key":"harness","value":"\(harness)","text":"","source":"\(harness) is installed"}
            ],"agents":["\(harness)"],"took_ms":840}
            """.utf8)
    }

    static let workspace = WorkspaceSummary(
        id: "ws-main", name: "Main", taskPrefix: "hb", isMain: true, ordinal: 0, repository: "repo-1",
        charter: "/fc/workspaces/ws-main/charter.md")

    /// What a setup did, in order.
    final class Log {
        var calls: [String] = []
        var started: [(OrchestratorHarness, CharterStart)] = []
        var closed = 0
    }

    static func setup(
        survey: Data? = survey(), surveyMessage: String? = nil,
        written: Data? = Data(#"{"workspace":"ws-main","wrote":true,"draft":false,"text":"x"}"#.utf8),
        writeMessage: String? = nil, installed: [OrchestratorHarness] = [.claude, .codex],
        draft: Bool = false, preferred: OrchestratorHarness? = nil, repository: String = "harbor", log: Log = Log(),
        stall: Bool = false, writeStall: Bool = false
    ) -> CharterSetup {
        CharterSetup(
            host: "", workspace: workspace, repository: repository, installed: installed, draftWaiting: draft,
            preferred: preferred,
            actions: CharterSetup.Actions(
                survey: {
                    log.calls.append("survey")
                    // Still reading when drawn: the view going away cancels it.
                    if stall { try? await Task.sleep(for: .seconds(60)) }
                    return (survey, surveyMessage)
                },
                write: { args in
                    log.calls.append(args.joined(separator: " "))
                    if writeStall { try? await Task.sleep(for: .seconds(60)) }
                    return (written, writeMessage)
                },
                start: { harness, how in log.started.append((harness, how)) },
                close: { log.closed += 1 }))
    }

    // MARK: - When the screens open

    @Test("Only this Mac's workspace with no charter, on a runner that surveys, opens the screens")
    func need() {
        let none: (String) -> Bool = { _ in false }
        let charter: (String) -> Bool = { $0.hasSuffix("/charter.md") }
        let draft: (String) -> Bool = { $0.hasSuffix("/charter-draft.md") }
        let ws = Self.workspace
        #expect(CharterNeed.of(ws, host: "", surveys: true, exists: none) == .charter)
        #expect(CharterNeed.of(ws, host: "", surveys: true, exists: draft) == .draft)
        #expect(CharterNeed.of(ws, host: "", surveys: true, exists: charter) == .none)
        // A runner from before the survey starts as it did, and the orchestrator asks.
        #expect(CharterNeed.of(ws, host: "", surveys: false, exists: none) == .none)
        // Another runner's disk can't be looked at from here.
        #expect(CharterNeed.of(ws, host: "carl.example", surveys: true, exists: none) == .none)
        // A runner that didn't say where the charter is, and a layout's implicit workspace.
        let unsaid = WorkspaceSummary(id: "w", name: "Main", taskPrefix: "hb", isMain: true, ordinal: 0)
        #expect(CharterNeed.of(unsaid, host: "", surveys: true, exists: none) == .none)
        #expect(CharterNeed.of(.implicit(repository: "r"), host: "", surveys: true, exists: none) == .none)
    }

    // MARK: - Reading, then the Charter

    @Test("Reading runs the survey, then the Charter screen has its rows and Codex, the agent it found")
    func reads() async {
        let log = Log()
        let setup = Self.setup(log: log)
        #expect(setup.phase == .reading && !setup.canStart)
        await setup.read()
        #expect(setup.phase == .ready)
        #expect(setup.form?.rows.first?.provenance == .from("main requires pull requests"))
        #expect(setup.harness == .codex)
        #expect(setup.canStart)
        #expect(log.calls == ["survey"])
    }

    @Test("An agent chosen from a menu stays chosen; one that isn't installed doesn't")
    func preferred() async {
        let chosen = Self.setup(preferred: .claude)
        await chosen.read()
        #expect(chosen.harness == .claude)
        let missing = Self.setup(installed: [.claude], preferred: .cursor)
        await missing.read()
        #expect(missing.harness == .claude, "the survey's codex isn't installed either, so the first that is")
    }

    @Test("A survey that didn't come back says why, and offers the start without it")
    func failedRead() async {
        let log = Log()
        let setup = Self.setup(survey: nil, surveyMessage: "error: no\ncode: capability-unsupported", log: log)
        await setup.read()
        guard case .failed(let why) = setup.phase else {
            Issue.record("\(setup.phase)")
            return
        }
        #expect(why.hasPrefix("Far Cooler couldn’t read harbor."), "\(why)")
        #expect(!why.contains("code:"), "no raw error on screen: \(why)")
        setup.startWithoutCharter()
        #expect(log.started.map(\.0) == [.claude] && log.started.map(\.1) == [.interview])
        #expect(log.calls == ["survey"], "nothing is written")
        #expect(log.closed == 1)
    }

    @Test("Leaving while it reads isn't a failure: it's still reading, and reads again on return")
    func leftWhileReading() async {
        let log = Log()
        let setup = Self.setup(log: log, stall: true)
        let reading = Task { await setup.read() }
        while log.calls.isEmpty { await Task.yield() }
        reading.cancel()
        await reading.value
        #expect(setup.phase == .reading, "\(setup.phase)")
        // Cancelled by the runner's own word too, as `runRaw` answers.
        let worded = Self.setup(survey: nil, surveyMessage: DaemonClient.cancelledMessage)
        await worded.read()
        #expect(worded.phase == .reading, "\(worded.phase)")
    }

    // MARK: - Start Orchestrator and Talk It Through

    @Test("Start Orchestrator writes the charter with what was changed, then starts the agent chosen")
    func start() async {
        let log = Log()
        let setup = Self.setup(log: log)
        await setup.read()
        setup.form?.choices.lanes = .noLimit
        setup.harness = .claude
        await setup.start(talk: false)
        #expect(log.calls == ["survey", "workspace write-charter ws-main --landing pull-requests --lanes none --json"])
        #expect(log.started.map(\.0) == [.claude] && log.started.map(\.1) == [.charter])
        #expect(log.closed == 1)
    }

    @Test("Talk It Through writes the draft and starts the orchestrator on it")
    func talk() async {
        let log = Log()
        let setup = Self.setup(log: log)
        await setup.read()
        await setup.start(talk: true)
        #expect(log.calls.last == "workspace write-charter ws-main --draft --landing pull-requests --json")
        #expect(log.started.map(\.1) == [.draft])
    }

    @Test("A charter that already exists isn't overwritten, and the orchestrator starts on it, saying so")
    func alreadyThere() async {
        let log = Log()
        let setup = Self.setup(written: Data(#"{"wrote":false,"draft":false}"#.utf8), log: log)
        await setup.read()
        await setup.start(talk: false)
        #expect(log.started.count == 1)
        // Not "the charter you just set": it's the one that was there (RN3).
        #expect(log.started.map(\.1) == [.existing])
        #expect(CharterStart.existing.startingLine == FirstRunCopy.Conversation.startingOnExisting)
    }

    @Test("Suggest Themes says why it didn't ask, by what stopped it")
    func undeliveredWords() {
        #expect(SuggestThemes.undelivered(.askedFirst) == FirstRunCopy.Charter.suggestAskedFirst)
        #expect(SuggestThemes.undelivered(.gone) == FirstRunCopy.Charter.suggestGone)
        #expect(SuggestThemes.undelivered(.neverReady) == FirstRunCopy.Charter.suggestUndelivered)
        #expect(SuggestThemes.undelivered(.notTyped) == FirstRunCopy.Charter.suggestUndelivered)
        #expect(SuggestThemes.undelivered(.notEmpty) == FirstRunCopy.Charter.suggestNotEmpty)
        #expect(SuggestThemes.undelivered(.mayHaveTyped) == FirstRunCopy.Charter.suggestCheckPane)
        // Only a prompt that may have gone in is not offered again.
        for cause: TaskFailure.Undelivered in [.askedFirst, .neverReady, .gone, .notTyped, .notEmpty] {
            #expect(SuggestThemes.offersAgain(after: cause), "\(cause)")
        }
        #expect(!SuggestThemes.offersAgain(after: .mayHaveTyped))
    }

    @Test("Starting… takes the pressed button's place: the action row doesn't move")
    func startingKeepsTheRow() async {
        let ready = Self.setup()
        await ready.read()
        let before = await EmptyStateProbe.draw(CharterSetupView(setup: ready), width: 1100, height: 860)
        let starting = Self.setup(writeStall: true)
        await starting.read()
        let pressed = Task { await starting.start(talk: false) }
        while starting.phase != .starting(talk: false) { await Task.yield() }
        let after = await EmptyStateProbe.draw(CharterSetupView(setup: starting), width: 1100, height: 860)
        pressed.cancel()
        #expect(after.contains("charter-starting"), "\(after.ids)")
        for id in ["charter-run-with", "charter-start", "charter-talk", "charter-not-now"] {
            #expect(before.frame(id) == after.frame(id), "\(id): \(String(describing: before.frame(id))) then \(String(describing: after.frame(id)))")
        }
    }

    @Test("A charter that couldn't be written says so and starts nothing")
    func writeFailed() async {
        let log = Log()
        let setup = Self.setup(written: nil, writeMessage: "error: refused", log: log)
        await setup.read()
        await setup.start(talk: false)
        #expect(log.started.isEmpty && log.closed == 0)
        #expect(setup.phase == .ready)
        #expect(setup.failure?.hasPrefix("Far Cooler couldn’t save harbor’s charter.") == true, "\(setup.failure ?? "")")
    }

    @Test("Continue Talking It Through starts on the draft that's there, and writes nothing")
    func resume() async {
        let log = Log()
        let setup = Self.setup(draft: true, log: log)
        await setup.read()
        setup.continueTalking()
        #expect(log.calls == ["survey"])
        #expect(log.started.map(\.1) == [.draft])
    }

    @Test("No agent installed: nothing can start")
    func noAgent() async {
        let setup = Self.setup(installed: [])
        await setup.read()
        #expect(setup.harness == nil && !setup.canStart)
    }

    // MARK: - The runner

    @Test("The survey and the charter go to the CLI by the repository's and workspace's ids")
    func cli() async {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        var asked: [[String]] = []
        client.commandRunnerForTesting = { args in
            asked.append(args)
            if args == ["--json", "status"] {
                return (Data(#"{"daemonVersion":"1","suggestThemesPrompt":"Suggest themes for this board."}"#.utf8), nil)
            }
            return (Data("{}".utf8), nil)
        }
        _ = await client.surveyRepository("repo-1")
        _ = await client.writeCharter(["workspace", "write-charter", "ws-main", "--json"])
        #expect(asked.prefix(2) == [["repo", "survey", "--repo", "repo-1", "--json"], ["workspace", "write-charter", "ws-main", "--json"]])
        // And what Suggest Themes sends, read with the runner's status.
        #expect(client.suggestThemesPrompt.isEmpty)
        await client.recheckInstalled()
        #expect(client.suggestThemesPrompt == "Suggest themes for this board.")
    }

    // MARK: - Suggest Themes

    @Test("Suggest Themes sends one line, and nothing for an older runner's empty prompt")
    func sendable() {
        #expect(SuggestThemes.sendable("") == nil)
        #expect(SuggestThemes.sendable("  \n ") == nil)
        #expect(SuggestThemes.sendable("Suggest themes.\nAsk me.") == "Suggest themes. Ask me.")
        #expect(SuggestThemes.sendable("a\u{1b}[2Jb") == "a [2Jb")
    }

    private static func invite(suggest: SuggestThemes?) -> OrchestratorInvite {
        OrchestratorInvite(
            seated: true, canAct: true, availability: HarnessAvailability(agentsFound: nil), place: .thisMac,
            onStart: { _ in }, suggestThemes: suggest)
    }

    @Test("An empty plan with the orchestrator running offers Suggest Themes once")
    func suggestOnPlan() async {
        let offered = await EmptyStateProbe.draw(
            PlanEmptyNotice().environment(\.orchestratorInvite, Self.invite(suggest: SuggestThemes(asked: false, send: {}))))
        #expect(offered.contains("plan-suggest-themes"), "\(offered.ids)")
        let asked = await EmptyStateProbe.draw(
            PlanEmptyNotice().environment(\.orchestratorInvite, Self.invite(suggest: SuggestThemes(asked: true, send: {}))))
        #expect(asked.contains("plan-themes-asked") && !asked.contains("plan-suggest-themes"), "\(asked.ids)")
        let older = await EmptyStateProbe.draw(PlanEmptyNotice().environment(\.orchestratorInvite, Self.invite(suggest: nil)))
        #expect(!older.contains("plan-suggest"), "\(older.ids)")
    }

    // MARK: - What's drawn

    @Test("Reading lists what it reads while the survey runs")
    func drawsReading() async {
        let drawn = await EmptyStateProbe.draw(CharterSetupView(setup: Self.setup(stall: true)), width: 1100, height: 700)
        #expect(drawn.contains("charter-reading"), "\(drawn.ids)")
        #expect(drawn.count("charter-reading-row") == FirstRunCopy.Charter.readingRows.count)
    }

    @Test("The Charter screen draws each row with where it came from, and the two ways to start")
    func drawsCharter() async {
        let setup = Self.setup()
        await setup.read()
        let drawn = await EmptyStateProbe.draw(CharterSetupView(setup: setup), width: 1100, height: 900)
        for id in ["workflow", "done_means", "review", "autonomy", "lanes", "initiative", "also"] {
            #expect(drawn.contains("charter-row-\(id)") && drawn.contains("charter-provenance-\(id)"), "\(id): \(drawn.ids)")
        }
        #expect(["charter-start", "charter-talk", "charter-not-now", "charter-run-with"].allSatisfy(drawn.contains), "\(drawn.ids)")
        #expect(!drawn.contains("charter-resume"))
    }

    @Test("A draft waiting offers to carry on with it")
    func drawsResume() async {
        let setup = Self.setup(draft: true)
        await setup.read()
        let drawn = await EmptyStateProbe.draw(CharterSetupView(setup: setup), width: 1100, height: 900)
        #expect(drawn.contains("charter-resume") && drawn.contains("charter-resume-button"), "\(drawn.ids)")
    }

    @Test("A failed survey offers Try Again and the start without it")
    func drawsFailure() async {
        let setup = Self.setup(survey: nil)
        await setup.read()
        let drawn = await EmptyStateProbe.draw(CharterSetupView(setup: setup), width: 1100, height: 700)
        #expect(["charter-read-failed", "charter-try-again", "charter-start-anyway"].allSatisfy(drawn.contains), "\(drawn.ids)")
    }

    @Test("A long name and long survey text keep Start Orchestrator on screen, unscrolled, at 1100 by 860")
    func longContent() async {
        let long = String(repeating: "Runs the integration suite against every supported database. ", count: 30)
        let setup = Self.setup(
            survey: Self.survey(text: long), repository: "harbor-incident-timeline-and-status-pages-service-monorepo")
        await setup.read()
        let (width, height): (CGFloat, CGFloat) = (1100, 860)
        let drawn = await EmptyStateProbe.draw(CharterSetupView(setup: setup), width: width, height: height)
        // Taller than the window, so the answers scroll; the actions don't.
        let rows = drawn.frame("charter-rows")
        #expect((rows?.maxY ?? 0) > height, "the answers fit, so this proves nothing: \(String(describing: rows))")
        for id in ["charter-start", "charter-talk", "charter-not-now", "charter-run-with"] {
            let frame = drawn.frame(id)
            #expect(
                frame.map { $0.minX >= 0 && $0.maxX <= width && $0.minY >= 0 && $0.maxY <= height } == true,
                "\(id) at \(String(describing: frame))")
        }
        // The long survey text wraps inside its row.
        let row = drawn.frame("charter-row-done_means")
        #expect(row.map { $0.maxX <= width } == true, "\(String(describing: row))")
    }
}
