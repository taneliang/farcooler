import Foundation
import Testing

@testable import AgentKit

/// The iPhone's follow-ups to 4A (ov-66): the stack reopening where it was,
/// a decision push landing on its task, and answering a task's question. In
/// AgentKit for `PhoneNavigationTests`' reason: the iOS target has no unit
/// tests, and each of these looks fine in a screenshot when it's wrong.
struct PhoneFollowUpTests {
    static let place = PhoneWorkspace(runner: "RUNNER-A", workspace: "ws-billing")
    static let saved: [PhoneRoute] = [
        .workspace(place), .task(place, task: "t7"),
        .worktree(runner: "RUNNER-A", worktree: "webhooks", landing: .terminal("agent")),
    ]

    static func decide(
        _ runners: [PhoneLaunch.Reading] = [.read], elapsed: TimeInterval = 1,
        moved: Bool = false, linking: Bool = false, items: Int = 2
    ) -> PhoneLaunch.Decision {
        PhoneLaunch.decide(
            runners, elapsed: elapsed, moved: moved, linking: linking, itemCount: items,
            last: place, exists: { _ in true })
    }

    // MARK: - Reopening where it was

    /// **A launch with nothing saved is ruling 4's**: Needs You while it
    /// holds anything, else the last workspace over it. A saved stack is
    /// `DestinationResolver`'s, not this rule's (`PhoneDestinationTests`).
    @Test("With nothing saved, a launch opens on Needs You or the last workspace")
    func aLaunchWithNothingSaved() {
        #expect(Self.decide(items: 2) == .open([]))
        #expect(Self.decide(items: 0) == .open([.workspace(Self.place)]))
        // Every runner first, as ever; and a link or a move still wins.
        #expect(Self.decide([.read, .waiting]) == .wait)
        #expect(Self.decide(linking: true) == .stay)
        #expect(Self.decide(moved: true) == .stay)
    }

    /// **The stack survives its own encoding**, every route and landing, and
    /// one this build can't read is no stack rather than part of one.
    @Test("A kept stack reads back as it was written, or not at all")
    func aKeptStackRoundTrips() throws {
        let data = try #require(PhoneLaunch.encode(Self.saved))
        #expect(PhoneLaunch.decode(data) == Self.saved)
        let changes: [PhoneRoute] = [
            .worktree(runner: "R", worktree: "w", landing: .changes),
            .worktree(runner: "R", worktree: "w", landing: .resume),
        ]
        #expect(PhoneLaunch.decode(PhoneLaunch.encode(changes)) == changes)
        #expect(PhoneLaunch.decode(nil) == [])
        #expect(PhoneLaunch.decode(Data(#"[{"board":{}}]"#.utf8)) == [])
    }

    // MARK: - A decision push

    static func decisionItem(workspace: String?) -> NeedsYouItem {
        NeedsYouItem(
            id: "decision:t7", kind: .decision, rank: 1, since: nil, workspaceID: workspace,
            repositoryID: "repo-1",
            task: NeedsYouTask(id: "t7", key: "bil-7", title: "Pick", status: "needs_decision"),
            question: "Which?", runner: "RUNNER-B")
    }

    /// **A decision push opens its task, over its workspace**, found by key
    /// on whichever runner has it: its Needs You item first, then a board.
    @Test("A decision push lands on its task, with the workspace under it")
    func aDecisionPushLandsOnItsTask() {
        let item = Self.decisionItem(workspace: "ws-billing")
        let quiet = PhoneDecisionLink.Source(runner: "RUNNER-A", items: [], boards: [:], implicit: false)
        let holding = PhoneDecisionLink.Source(
            runner: "RUNNER-B", items: [item], boards: [:], implicit: false)
        let place = PhoneWorkspace(runner: "RUNNER-B", workspace: "ws-billing")
        #expect(
            PhoneDecisionLink.find(Self.push("bil-7"), in: [quiet, holding])
                == [.workspace(place), .task(place, task: "t7")])
        #expect(PhoneDecisionLink.find(Self.push("bil-8"), in: [quiet, holding]) == nil)
        #expect(PhoneDecisionLink.find(Self.push(""), in: [quiet, holding]) == nil)

        // On a runner without workspaces, the repository's board.
        let implicit = PhoneDecisionLink.Source(
            runner: "RUNNER-B", items: [Self.decisionItem(workspace: nil)], boards: [:],
            implicit: true)
        let repo = PhoneWorkspace(runner: "RUNNER-B", workspace: "repo-1")
        #expect(
            PhoneDecisionLink.find(Self.push("bil-7"), in: [implicit])
                == [.workspace(repo), .task(repo, task: "t7")])

        // Answered already, so off Needs You: its card on a board still has it.
        let row = TaskRow(
            id: "t7", key: "bil-7", title: "Pick", status: .needsDecision, statusSince: Date())
        let board = TaskBoardModel(columns: [TaskBoardColumn(status: .needsDecision, rows: [row])])
        let boards = PhoneDecisionLink.Source(
            runner: "RUNNER-A", items: [], boards: ["ws-billing": board], implicit: false)
        #expect(
            PhoneDecisionLink.find(Self.push("bil-7"), in: [boards])
                == [.workspace(Self.place), .task(Self.place, task: "t7")])
    }

    static func push(_ key: String, runner: String? = nil) -> DecisionPush {
        DecisionPush(key: key, runner: runner)
    }

    /// **Two runners with a task under one key are never guessed between.**
    /// A push naming its runner lands on that runner's; one naming none
    /// lands nowhere, rather than on whichever runner was asked first.
    @Test("A decision push lands on the runner it names, and never guesses between two")
    func aDecisionPushNeverGuessesBetweenRunners() {
        func source(_ runner: String) -> PhoneDecisionLink.Source {
            var item = Self.decisionItem(workspace: "ws-\(runner)")
            item.runner = runner
            // The phone's own id for a runner is not the id a push names: that
            // is the daemon's `Host.runner_id`, lowercase (ov-72).
            return PhoneDecisionLink.Source(
                runner: runner, items: [item], boards: [:], implicit: false,
                hostRunner: "host-\(runner.lowercased())")
        }
        let a = source("RUNNER-A")
        let b = source("RUNNER-B")
        var unread = source("RUNNER-A")
        unread.hostRunner = nil
        let onB = PhoneWorkspace(runner: "RUNNER-B", workspace: "ws-RUNNER-B")
        #expect(
            PhoneDecisionLink.find(Self.push("bil-7", runner: "host-runner-b"), in: [a, b])
                == [.workspace(onB), .task(onB, task: "t7")])
        // The push says the id the daemon minted, however it is cased.
        #expect(
            PhoneDecisionLink.find(Self.push("bil-7", runner: "HOST-runner-b"), in: [a, b])
                == [.workspace(onB), .task(onB, task: "t7")])
        // Naming the phone's own id for a runner names nobody.
        #expect(PhoneDecisionLink.find(Self.push("bil-7", runner: "RUNNER-B"), in: [a, b]) == nil)
        // A runner whose id hasn't been read yet is not the one named.
        #expect(PhoneDecisionLink.find(Self.push("bil-7", runner: "host-runner-a"), in: [unread]) == nil)
        // While the wait is on, a runner nobody knows is nobody. Once it has
        // ended, the key is looked for on its own: exactly one runner has it,
        // so that one opens; two, and it's Needs You still.
        let onA = PhoneWorkspace(runner: "RUNNER-A", workspace: "ws-RUNNER-A")
        #expect(
            PhoneDecisionLink.find(Self.push("bil-7", runner: "host-runner-a"), in: [unread], waitEnded: true)
                == [.workspace(onA), .task(onA, task: "t7")])
        #expect(
            PhoneDecisionLink.find(Self.push("bil-7", runner: "host-gone"), in: [a], waitEnded: true)
                == [.workspace(onA), .task(onA, task: "t7")])
        #expect(PhoneDecisionLink.find(Self.push("bil-7", runner: "host-gone"), in: [a, b], waitEnded: true) == nil)
        // A runner that is known, and doesn't have the key, is not fallen back
        // from: the task is not there, whatever the other runner holds.
        var bare = b
        bare.items = []
        #expect(PhoneDecisionLink.find(Self.push("bil-7", runner: "host-runner-b"), in: [a, bare], waitEnded: true) == nil)
        #expect(PhoneDecisionLink.find(Self.push("bil-7"), in: [a, b]) == nil)
        #expect(PhoneDecisionLink.find(Self.push("bil-7", runner: "host-runner-c"), in: [a, b]) == nil)
    }

    // MARK: - Answering, the one task write (ov-184)

    /// **An answer is a `task.note` of kind `answer`**, the one kind the
    /// client core still takes from a phone: anything else it refuses before
    /// the runner sees it, and the waiting agent would never wake.
    @Test("An answer goes out as a task note of kind answer")
    func anAnswerIsATaskNoteOfKindAnswer() {
        #expect(PhoneTaskAnswer.method == "task.note")
        #expect(
            PhoneTaskAnswer.request(task: "0192-task", body: "Postgres")
                == ["task": "0192-task", "kind": "answer", "body": "Postgres"])
    }

    /// **No iPhone, Watch or extension source names a task write the
    /// orchestrator owns** (ov-184), so no screen can reach one through the
    /// client core, which has no arm for them either
    /// (`no_phone_can_write_a_task` in `crates/client`). The sources are
    /// `apps/ios` and the AgentKit files those targets compile in
    /// (`PhoneSources.agentKit`), where iOS's answer is named.
    @Test("No iOS source names task.create, task.update or task.set_status")
    func noPhoneSourceNamesATaskWrite() throws {
        let ios = PhoneSources.apps.appendingPathComponent("ios")
        let walker = try #require(FileManager.default.enumerator(at: ios, includingPropertiesForKeys: nil))
        let app = walker.compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" }
        #expect(app.count > 50, "found \(app.count) sources under \(ios.path), so this proves nothing")
        let agentKit = try PhoneSources.agentKit()
        #expect(agentKit.count > 50, "found \(agentKit.count) phone-compiled AgentKit sources")
        let texts = try (app + agentKit).map {
            ($0.lastPathComponent, try String(contentsOf: $0, encoding: .utf8))
        }
        // Proves the scan reads string literals, in both halves: the board's
        // read where the app names it, and the answer's in AgentKit.
        #expect(texts.contains { $0.1.contains("\"task.get\"") })
        #expect(texts.contains { $0.0 == "ShellNavigation.swift" && $0.1.contains("\"task.note\"") })

        let named = PhoneSources.taskWrites(in: texts)
        #expect(named.isEmpty, "a task write the orchestrator owns: \(named)")
        let all = PhoneSources.taskWrites(in: texts, exempting: [])
        let stale = PhoneSources.namedButNotSent.subtracting(all)
        #expect(stale.isEmpty, "no longer named, so drop them from namedButNotSent: \(stale)")
    }

    /// **A task write planted in AgentKit's phone code is found**: a listed
    /// file is read, and a file on no list isn't.
    @Test("A task write in a phone-compiled AgentKit file is found")
    func aPlantedAgentKitTaskWriteIsFound() throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("ov184-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        try "let m = \"task.create\"\n".write(
            to: dir.appendingPathComponent("Planted.swift"), atomically: true, encoding: .utf8)
        try "let m = \"task.update\"\n".write(
            to: dir.appendingPathComponent("MacOnly.swift"), atomically: true, encoding: .utf8)
        let generator = """
            AGENTKIT_SOURCES = [
                # "MacOnly.swift" is not here
            ]
            WATCH_AGENTKIT_SOURCES = [
                "Planted.swift",
            ]
            WATCH_WIDGET_AGENTKIT_SOURCES = [
            ]
            """
        let files = try PhoneSources.agentKit(generator: generator, sources: dir)
        #expect(files.map(\.lastPathComponent) == ["Planted.swift"])
        let texts = try files.map { ($0.lastPathComponent, try String(contentsOf: $0, encoding: .utf8)) }
        #expect(PhoneSources.taskWrites(in: texts) == ["task.create in Planted.swift"])
    }
}

/// The sources the iPhone, its extensions, the watch and its widget compile,
/// for the task-write guard above.
enum PhoneSources {
    /// `…/apps`, from this file's path.
    static var apps: URL {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return root
    }

    /// The task writes the orchestrator owns, which no phone names.
    static let writes = ["task.create", "task.update", "task.set_status", "task.move", "task.block"]

    /// Named in phone-compiled code and never sent, until the Mac's half of
    /// ov-184 removes it: `TaskBoardModel.moves` is the Mac board's menu. The
    /// same entry as `NAMED_BUT_NOT_SENT` in `crates/client`.
    static let namedButNotSent: Set<String> = ["task.set_status in TaskBoardModel.swift"]

    /// Each task write a source in `texts` names, as "method in file", but
    /// for `exempt`.
    static func taskWrites(
        in texts: [(String, String)], exempting exempt: Set<String> = namedButNotSent
    ) -> [String] {
        texts.flatMap { name, text in
            writes.filter { text.contains("\"\($0)\"") }.map { "\($0) in \(name)" }
        }.filter { !exempt.contains($0) }
    }

    /// AgentKit's sources that the phone's targets compile, by the three lists
    /// in `apps/ios/generate-project.py` that build them: iOS has no SwiftPM
    /// project, and compiles exactly these files.
    static func agentKit(generator: String? = nil, sources: URL? = nil) throws -> [URL] {
        let text = try generator
            ?? String(contentsOf: apps.appendingPathComponent("ios/generate-project.py"), encoding: .utf8)
        let dir = sources ?? apps.appendingPathComponent("shared/AgentKit/Sources/AgentKit")
        var names = Set<String>()
        for list in ["AGENTKIT_SOURCES", "WATCH_AGENTKIT_SOURCES", "WATCH_WIDGET_AGENTKIT_SOURCES"] {
            let lines = text.components(separatedBy: "\n")
            let start = try #require(lines.firstIndex(of: "\(list) = ["), "generate-project.py has no \(list)")
            let end = try #require(lines[start...].firstIndex(of: "]"), "\(list) never closes")
            for line in lines[(start + 1)..<end]
            where !line.trimmingCharacters(in: .whitespaces).hasPrefix("#") {
                let parts = line.components(separatedBy: "\"")
                names.formUnion(stride(from: 1, to: parts.count, by: 2).map { parts[$0] }
                    .filter { $0.hasSuffix(".swift") })
            }
        }
        return names.sorted().map { dir.appendingPathComponent($0) }
    }
}
