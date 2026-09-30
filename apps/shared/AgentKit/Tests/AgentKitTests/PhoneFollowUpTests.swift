import Foundation
import Testing

@testable import AgentKit

/// The iPhone's follow-ups to 4A (ov-66): the stack reopening where it was,
/// a decision push landing on its task, and New Task… on a board. In
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
        moved: Bool = false, linking: Bool = false, items: Int = 2,
        saved: [PhoneRoute] = saved,
        presence: @escaping (PhoneRoute) -> PhoneLaunch.Presence = { _ in .here }
    ) -> PhoneLaunch.Decision {
        PhoneLaunch.decide(
            runners, elapsed: elapsed, moved: moved, linking: linking, itemCount: items,
            last: place, exists: { _ in true }, saved: saved, presence: presence)
    }

    // MARK: - Reopening where it was

    /// **A relaunch reopens the stack it was killed on**: workspace, task and
    /// worktree, over Needs You, even with items waiting there (ruling 1).
    @Test("A saved stack reopens as it was, whatever Needs You holds")
    func aSavedStackReopens() {
        #expect(Self.decide() == .open(Self.saved))
        #expect(Self.decide(items: 0) == .open(Self.saved))
        // Nothing saved: ruling 4 decides, as before.
        #expect(Self.decide(items: 2, saved: []) == .open([]))
        #expect(Self.decide(items: 0, saved: []) == .open([.workspace(Self.place)]))
    }

    /// **A saved stack with any screen gone falls back to Needs You**, and
    /// says nothing; one still being read waits, until the limit.
    @Test("A saved stack whose screen is gone falls back to Needs You")
    func aGoneScreenFallsBackToNeedsYou() {
        let taskGone: (PhoneRoute) -> PhoneLaunch.Presence = {
            if case .task = $0 { return .gone }
            return .here
        }
        #expect(Self.decide(presence: taskGone) == .stay)
        #expect(Self.decide(items: 0, presence: taskGone) == .stay)
        let boardUnread: (PhoneRoute) -> PhoneLaunch.Presence = {
            if case .task = $0 { return .unknown }
            return .here
        }
        #expect(Self.decide(presence: boardUnread) == .wait)
        #expect(Self.decide(elapsed: 10, presence: boardUnread) == .stay)
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
        #expect(
            PushTap(userInfo: ["kind": "decision", "task": "bil-7", "runner": "host-runner-b"], thread: "")
                == .task(Self.push("bil-7", runner: "host-runner-b")))
    }

    /// **A push waits for its runners, not ten seconds**: on a cold launch
    /// over a slow network it keeps looking until every runner has said
    /// what needs you and read its boards, or a minute has gone.
    @Test("A decision push waits until the runners have settled, or a minute")
    func aDecisionPushWaitsForTheRunners() {
        #expect(!PhoneDecisionLink.givesUp(settled: false, elapsed: 30))
        #expect(!PhoneDecisionLink.givesUp(settled: false, elapsed: 59))
        #expect(PhoneDecisionLink.givesUp(settled: false, elapsed: 60))
        #expect(PhoneDecisionLink.givesUp(settled: true, elapsed: 1))
    }

    /// **A tap says which it is**: a decision's task by key, else an agent's
    /// terminal, from the push or from the banner this app posted.
    @Test("A tapped notification is a decision's task or an agent's terminal")
    func aTapIsATaskOrATerminal() {
        #expect(
            PushTap(userInfo: ["kind": "decision", "task": "bil-7", "terminal": ""], thread: "")
                == .task(DecisionPush(key: "bil-7", runner: nil)))
        #expect(PushTap(userInfo: ["terminal": "d002", "status": "blocked"], thread: "x") == .terminal("d002"))
        #expect(PushTap(userInfo: [:], thread: "d003") == .terminal("d003"))
        #expect(PushTap(userInfo: ["kind": "decision"], thread: "d004") == .terminal("d004"))
        #expect(PushTap(userInfo: [:], thread: "") == nil)
    }

    // MARK: - New Task…

    /// **A title is counted in Unicode scalars, 200 at most**, as the runner
    /// counts it: 199 flags' worth of characters is 398 scalars.
    @Test("A title fits in 200 Unicode scalars, trimmed and not empty")
    func aTitleFitsInTwoHundredScalars() {
        #expect(PhoneNewTask.titleFits(String(repeating: "a", count: 200)))
        #expect(!PhoneNewTask.titleFits(String(repeating: "a", count: 201)))
        #expect(PhoneNewTask.titleFits("  " + String(repeating: "a", count: 200) + "\n"))
        #expect(!PhoneNewTask.titleFits("   "))
        // 101 flags: 101 characters, 202 scalars.
        #expect(!PhoneNewTask.titleFits(String(repeating: "🇸🇬", count: 101)))
        #expect(PhoneNewTask.titleFits(String(repeating: "🇸🇬", count: 100)))
        #expect(PhoneNewTask.isTooLong(String(repeating: "a", count: 201)))
        #expect(!PhoneNewTask.isTooLong(""))
    }

    /// **The arguments name the board**: a workspace's, or for an implicit
    /// one only its repository; the title trimmed, details as the intent.
    @Test("task.create names the workspace's board, and details are the intent")
    func theRequestNamesTheBoard() {
        let billing = WorkspaceSummary(
            id: "ws-billing", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1,
            repository: "repo-1")
        #expect(
            PhoneNewTask.request(billing, title: "  Ship it ", details: "")
                == ["repository": "repo-1", "workspace": "ws-billing", "title": "Ship it"])
        #expect(
            PhoneNewTask.request(.implicit(repository: "repo-1"), title: "Ship", details: " Why \n")
                == ["repository": "repo-1", "title": "Ship", "intent": "Why"])
    }

    /// **New Task… is hidden on a Read grant, and a refusal is a sentence**,
    /// the Mac's for a title too long, never the runner's word.
    @Test("New Task is offered below Read, and its refusals are sentences")
    func newTaskOfferAndRefusals() {
        func build(_ scope: String) -> DaemonBuild {
            DaemonBuild(
                version: "1", matches: true, platform: "p", capabilities: [], grantedScope: scope)
        }
        #expect(!PhoneNewTask.offered(build("read")))
        #expect(PhoneNewTask.offered(build("control")))
        #expect(PhoneNewTask.offered(build("unspecified")))
        #expect(PhoneNewTask.offered(nil))

        #expect(
            PhoneNewTask.refusal(word: "invalid-argument", what: "title")
                == "That title is too long. Shorten it to add the task.")
        #expect(
            PhoneNewTask.refusal(word: "scope-denied", what: nil)
                == "This device can only look at this runner, so it can’t add tasks.")
        #expect(
            PhoneNewTask.refusal(word: "not-found", what: nil)
                == "This board isn’t on the runner anymore.")
        #expect(
            PhoneNewTask.refusal(word: nil, what: nil)
                == "Couldn’t add that task. Check that the runner is reachable, then try again.")
        for word in ["capability-unsupported", "invalid-argument", "internal"] {
            let sentence = PhoneNewTask.refusal(word: word, what: nil)
            #expect(!sentence.contains(word), "a runner's word never reaches the screen")
            #expect(sentence.hasSuffix("."))
        }
    }
}
