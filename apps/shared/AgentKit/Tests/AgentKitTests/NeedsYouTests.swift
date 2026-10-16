import Foundation
import Testing

@testable import AgentKit

// The needs-you list as an app holds it: decoded from the one JSON shape the
// client core writes, merged across runners by rank, and counted per
// workspace. And, for a runner too old to send a list, the little that can be
// said without guessing.

/// `test/fixtures/needs-you.json`, which the Rust and Kotlin suites read too:
/// what `needs_you_json` writes for two runners, with the daemon's own values.
private struct Fixture: Decodable {
    struct Runner: Decodable {
        var runner: String
        var needs_you: NeedsYouList
    }
    var runners: [Runner]

    static func load() throws -> Fixture {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/needs-you.json"))
        return try JSONDecoder().decode(Fixture.self, from: data)
    }

    func items(of runner: String) -> [NeedsYouItem] {
        runners.first { $0.runner == runner }?.needs_you.items ?? []
    }
}

private func millis(_ ms: Int64) -> Date { Date(timeIntervalSince1970: TimeInterval(ms) / 1000) }

@Test("The shared fixture decodes to the values it holds")
func theSharedFixtureDecodesToTheValuesItHolds() throws {
    let fixture = try Fixture.load()
    #expect(fixture.runners.map(\.runner) == ["studio", "build-box"])

    let studio = fixture.items(of: "studio")
    #expect(studio.map(\.kind) == [.ask, .blocked, .decision, .prThread, .prReview, .review])

    let ask = studio[0]
    #expect(ask.itemID == "ask:hook-ask-01000000-0000-7000-8000-000000000029")
    #expect(ask.also == [.decision])
    #expect(ask.rank == 99_999_939)
    #expect(ask.since == millis(1_789_999_940_000))
    #expect(ask.workspaceID == "01000000-0000-7000-8000-000000000001")
    #expect(ask.workspaceName == "Billing")
    #expect(ask.repositoryID == "01000000-0000-7000-8000-000000000002")
    #expect(
        ask.task
            == NeedsYouTask(
                id: "01000000-0000-7000-8000-000000000003", key: "bil-7",
                title: "Invoice PDF export", status: "needs_decision"))
    #expect(
        ask.terminal
            == NeedsYouTerminal(
                id: "01000000-0000-7000-8000-000000000004",
                worktreeID: "01000000-0000-7000-8000-000000000005", label: "claude",
                role: "agent", paneMode: "terminal"))
    #expect(
        ask.worktree
            == NeedsYouWorktree(
                id: "01000000-0000-7000-8000-000000000005", name: "fc-3-webhooks",
                branch: "bil/webhooks", insertions: 18, deletions: 40))
    #expect(ask.question == "Allow touch x")
    #expect(ask.detail == nil)
    #expect(ask.askID == "hook-ask-01000000-0000-7000-8000-000000000029")
    #expect(
        ask.actions == [
            NeedsYouAction(id: "allow", title: "Allow touch x", destructive: false, primary: true),
            NeedsYouAction(id: "deny", title: "Deny", destructive: true, primary: false),
        ])

    let blocked = studio[1]
    #expect(blocked.itemID == "blocked:01000000-0000-7000-8000-00000000000a")
    #expect(blocked.also.isEmpty)
    #expect(blocked.rank == 199_999_879)
    #expect(blocked.since == millis(1_789_999_880_000))
    #expect(blocked.task == nil)
    #expect(blocked.terminal?.isOrchestrator == true)
    #expect(blocked.terminal?.label == "codex")
    #expect(blocked.worktree?.name == "demo")
    #expect(blocked.worktree?.branch == "main")
    #expect(blocked.question == "Run the migration now?")
    #expect(blocked.askID == nil)
    #expect(blocked.actions.map(\.id) == ["open"])
    #expect(blocked.actions.first?.isOpen == true)
    #expect(blocked.actions.first?.primary == false)

    let decision = studio[2]
    #expect(decision.itemID == "decision:01000000-0000-7000-8000-00000000000c")
    #expect(decision.rank == 299_999_699)
    #expect(decision.task?.key == "bil-9")
    #expect(decision.task?.title == "Retry failed webhooks")
    #expect(decision.terminal == nil)
    #expect(decision.worktree == nil)
    #expect(decision.question == "Postgres or SQLite for the queue?")
    #expect(decision.actions.map(\.id) == ["Postgres", "SQLite"])
    #expect(decision.actions.map(\.title) == ["Postgres", "SQLite"])

    // The two pull-request kinds (ov-315): a thread an agent escalated, which
    // is a decision about a thread, and a teammate's pull request.
    let thread = studio[3]
    #expect(thread.itemID == "pr-thread:01000000-0000-7000-8000-00000000000e")
    #expect(thread.rank == 399_999_099)
    #expect(thread.task?.key == "bil-11")
    #expect(thread.question == "alice asked, says the agent: “Why not retry on 5xx?”")
    #expect(thread.detail == "acme/billing #212")
    #expect(thread.link == "https://github.com/acme/billing/pull/212#discussion_r1")
    #expect(thread.actions.map(\.id) == ["Retry"])

    let request = studio[4]
    #expect(request.itemID == "pr-review:0123456789abcdef")
    #expect(request.rank == 499_998_199)
    #expect(request.task == nil && request.terminal == nil)
    #expect(request.workspaceID == nil, "a review request is in no workspace")
    #expect(request.question == "Add refund endpoint")
    #expect(request.detail == "acme/billing #213 · bob")
    #expect(request.link == "https://github.com/acme/billing/pull/213")
    #expect(request.actions.isEmpty)
    #expect(studio.filter { $0.kind != .prReview && $0.kind != .prThread }.allSatisfy { $0.link == nil })

    let review = studio[5]
    #expect(review.itemID == "review:01000000-0000-7000-8000-00000000000d")
    #expect(review.rank == 599_996_399)
    #expect(review.since == millis(1_789_996_400_000))
    #expect(review.task?.status == "in_review")
    #expect(review.question == "Ready for review")
    #expect(review.detail == "+18 −40")
    #expect(review.worktree?.insertions == 18)

    // The build box is read below Control: no worktree, detail, ask id or
    // actions, and a fixed sentence for a question.
    let box = fixture.items(of: "build-box")
    #expect(box.map(\.kind) == [.ask, .review])
    #expect(box[0].question == "claude is asking to use a tool")
    #expect(box[0].workspaceName == "Main")
    #expect(box[0].actions.isEmpty)
    #expect(box[0].askID == nil)
    #expect(box[0].worktree == nil)
    #expect(box[0].link == nil)
    #expect(box[1].workspaceID == nil, "null on the wire is no workspace")
    #expect(box[1].workspaceName == "")
    #expect(box[1].repositoryID == "01000000-0000-7000-8000-000000000015")
    #expect(box[1].task?.key == "ops-2")
}

@Test("Two runners merge by rank, not by clock")
func twoRunnersMergeByRankNotByClock() {
    // The build box's clock runs ten minutes slow, so its ask READS older than
    // the studio's. By rank it has waited less, which is the truth.
    let now: Int64 = 1_790_000_000_000
    let studio = NeedsYouItem(
        id: "ask:a", kind: .ask, rank: 99_999_939, since: millis(now - 60_000), question: "a")
    let box = NeedsYouItem(
        id: "ask:b", kind: .ask, rank: 99_999_989, since: millis(now - 600_000 - 10_000),
        question: "b")
    let review = NeedsYouItem(
        id: "review:c", kind: .review, rank: 399_999_999, since: millis(now - 9_000_000),
        question: "c")

    let merged = NeedsYou.merge(["build-box": [box, review], "studio": [studio]])
    #expect(merged.map(\.itemID) == ["ask:a", "ask:b", "review:c"])
    #expect(merged.map(\.runner) == ["studio", "build-box", "build-box"])

    // A tie in rank falls to the runner, so the order doesn't shuffle. The
    // ids run the other way, so dropping the runner's clause changes the
    // order every time rather than by the dictionary's luck.
    let tied = NeedsYou.merge([
        "studio": [NeedsYouItem(id: "ask:a", kind: .ask, rank: 5, since: .now, question: "")],
        "build-box": [NeedsYouItem(id: "ask:z", kind: .ask, rank: 5, since: .now, question: "")],
    ])
    #expect(tied.map(\.runner) == ["build-box", "studio"])

    let same = NeedsYou.merge([
        "studio": [NeedsYouItem(id: "ask:x", kind: .ask, rank: 5, since: .now, question: "")],
        "build-box": [NeedsYouItem(id: "ask:x", kind: .ask, rank: 5, since: .now, question: "")],
    ])
    #expect(Set(same.map(\.key)).count == 2, "one id on two runners is two items")
}

@Test("An unknown kind decodes and sorts last")
func anUnknownKindDecodesAndSortsLast() throws {
    let json = Data(
        """
        {"items": [{"id": "mystery:1", "kind": "mystery", "also": ["ask", "mystery"],
          "rank": 1, "since": 1789999940000, "workspace_id": null, "workspace_name": "",
          "repository_id": null, "task": null, "terminal": null, "worktree": null,
          "question": "Something new", "detail": null, "ask_id": null, "actions": []}]}
        """.utf8)
    let list = try JSONDecoder().decode(NeedsYouList.self, from: json)
    #expect(list.items.map(\.kind) == [.unknown])
    #expect(list.items[0].also == [.ask, .unknown])

    let review = NeedsYouItem(
        id: "review:r", kind: .review, rank: 399_999_999, since: .now, question: "r")
    let merged = NeedsYou.merge(["a": list.items, "b": [review]])
    #expect(merged.map(\.itemID) == ["review:r", "mystery:1"])
}

@Test("A workspace's count is its items, not its signals")
func aWorkspacesCountIsItsItemsNotItsSignals() throws {
    let studio = try Fixture.load().items(of: "studio")
    let billing = "01000000-0000-7000-8000-000000000001"
    // Five items in the workspace, one of which also carries a decision: five,
    // not six. The pull request that wants a review is in none.
    #expect(studio.count(in: billing) == 5)
    #expect(studio.count(in: "01000000-0000-7000-8000-000000000014") == 0)

    let box = try Fixture.load().items(of: "build-box")
    let merged = NeedsYou.merge(["studio": studio, "build-box": box])
    #expect(merged.count(in: billing) == 5)
    #expect(merged.count(in: "01000000-0000-7000-8000-000000000014") == 1)
    #expect(merged.unclaimedCount(inRepository: "01000000-0000-7000-8000-000000000015") == 1)
    #expect(merged.unclaimedCount(inRepository: "01000000-0000-7000-8000-000000000002") == 0)
}

private func pane(
    _ id: String, activity: String?, rank: UInt32?, question: String? = nil,
    workspace: String? = "w1", task: NeedsYouTask? = nil
) -> NeedsYou.OlderPane {
    NeedsYou.OlderPane(
        terminal: NeedsYouTerminal(
            id: id, worktreeID: "wt", label: "codex", role: "agent", paneMode: "terminal"),
        activity: activity, rank: rank, activitySince: millis(1_789_999_000_000),
        blockedQuestion: question, workspaceID: workspace, repositoryID: "r1", task: task,
        worktree: NeedsYouWorktree(
            id: "wt", name: "fc-3-webhooks", branch: "bil/webhooks", insertions: 0, deletions: 0))
}

@Test("An older runner's blocked agent is a blocked item, never above a real ask")
func anOlderRunnersBlockedAgentIsABlockedItemNeverAboveARealAsk() throws {
    // `Terminal.rank`'s tier 0 is Blocked: this agent has waited an hour.
    let old = NeedsYou.derived(fromTerminals: [
        pane("t1", activity: "blocked", rank: 99_996_399, question: "Trust this folder?"),
        pane(
            "t2", activity: "blocked", rank: nil, question: "",
            task: NeedsYouTask(id: "k", key: "bil-9", title: "Retry", status: "in_progress")),
    ])
    #expect(old.map(\.kind) == [.blocked, .blocked])
    #expect(old[0].itemID == "blocked:t1")
    #expect(old[0].rank == 199_996_399, "into the blocked tier, keeping its age")
    #expect(old[0].question == "Trust this folder?")
    #expect(old[0].workspaceID == "w1")
    #expect(old[0].terminal?.id == "t1")
    #expect(old[0].since == millis(1_789_999_000_000))
    #expect(old[0].actions.map(\.id) == ["open"])
    #expect(old[1].rank == 199_999_999, "no rank is the youngest in the tier")
    #expect(old.allSatisfy { $0.isDerived }, "built here, and marked so")
    #expect(old[1].question == "codex needs you", "an empty question is none")
    #expect(old[1].task?.key == "bil-9", "the task it was dispatched for")
    #expect(old[0].task == nil)
    #expect(old[0].worktree?.name == "fc-3-webhooks")

    // A current runner's ask, one minute old, still comes first.
    let ask = try Fixture.load().items(of: "studio")[0]
    let merged = NeedsYou.merge(["old-box": old, "studio": [ask]])
    #expect(merged.map(\.itemID) == [ask.itemID, "blocked:t1", "blocked:t2"])
}

@Test("An older runner derives no decisions or reviews")
func anOlderRunnerDerivesNoDecisionsOrReviews() {
    // A finished agent, a working one, an idle one, a shell, and a failed
    // turn: none of them is an item an older runner can vouch for.
    let derived = NeedsYou.derived(fromTerminals: [
        pane("done", activity: "done", rank: 150_000_000),
        pane("working", activity: "working", rank: 250_000_000),
        pane("idle", activity: "idle", rank: 350_000_000),
        pane("shell", activity: nil, rank: 350_000_000),
        pane("odd", activity: "unknown", rank: 350_000_000),
    ])
    #expect(derived.isEmpty)
    #expect(
        NeedsYou.olderRunnerNote(runner: "build-box")
            == "Update Far Cooler on build-box to see decisions and asks here.")
}

/// The phone's own fleet, read the way an older runner's items are derived:
/// its blocked pane, with the label, workspace and task the daemon would give.
@Test("The phone's fleet feeds the older-runner items")
func thePhonesFleetFeedsTheOlderRunnerItems() throws {
    let json = FleetDecodeTests.fleetJSON
        .replacingOccurrences(of: #""activity": "done""#, with: #""activity": "blocked""#)
        .replacingOccurrences(of: #""rank": 199999940"#, with: #""rank": 99999940"#)
        .replacingOccurrences(
            of: #""taskId": "0198f2c0-0000-7000-8000-00000000a001""#,
            with: #""taskId": "0198f2c0-0000-7000-8000-00000000a002""#)
    let fleet = try FleetDecodeTests.decodeFleet(json)
    let items = NeedsYou.derived(fromTerminals: fleet.olderPanes())
    let item = try #require(items.first)
    #expect(items.count == 1)
    #expect(item.itemID == "blocked:aab3238922bcc25a6f606eb525ffdc56")
    #expect(item.rank == 199_999_940, "Terminal.rank's tier 0, moved to the blocked tier")
    #expect(item.question == "Run `rm -rf build`?")
    #expect(item.terminal?.label == "claude")
    #expect(item.terminal?.worktreeID == "8f14e45f-ce5b-4a5e-9c2b-000000000001")
    #expect(item.terminal?.isOrchestrator == true)
    // The terminal's own workspace, over its worktree's owner.
    #expect(item.workspaceID == "0198f2c0-0000-7000-8000-0000000000cc")
    #expect(item.repositoryID == "1c383cd3-0b0f-4a63-b8a1-000000000002")
    #expect(item.task?.key == "bil-9")
    #expect(item.worktree?.name == "Widen the model")
    #expect(item.since == Date(timeIntervalSince1970: 1_755_900_000))
}

/// What a client core writes as null: a role it doesn't know (a newer
/// runner's), a terminal with no worktree id, and an item with no `since`.
/// Each costs its own field, never the runner's whole list.
@Test("Nulls the client core may write decode as absent")
func nullsTheClientCoreMayWriteDecodeAsAbsent() throws {
    let json = Data(
        """
        {"items": [{"id": "blocked:t", "kind": "blocked", "also": [], "rank": 100000001,
          "since": null, "workspace_id": null, "workspace_name": "", "repository_id": null,
          "task": null, "worktree": null, "question": "q", "detail": null, "ask_id": null,
          "actions": [],
          "terminal": {"id": "t", "worktree_id": null, "label": "codex", "role": null,
                       "pane_mode": "terminal"}}]}
        """.utf8)
    let item = try #require(try JSONDecoder().decode(NeedsYouList.self, from: json).items.first)
    #expect(item.since == nil)
    #expect(item.terminal?.role == nil)
    #expect(item.terminal?.worktreeID == nil)
    #expect(item.terminal?.isOrchestrator == false)
    #expect(!item.isDerived, "decoded from a runner's own list")
}

/// A pane whose runner didn't say when it blocked shows no age. `now` in its
/// place would read "just now" for an agent stuck an hour.
@Test("An older pane with no activity time has no since")
func anOlderPaneWithNoActivityTimeHasNoSince() {
    var blocked = pane("t", activity: "blocked", rank: 5)
    blocked.activitySince = nil
    let item = NeedsYou.derived(fromTerminals: [blocked])
    #expect(item.count == 1)
    #expect(item.first?.since == nil)
}

/// A command is said once on an ask: in the question, or under it, not both.
@Test("The detail is dropped when the question already says it")
func detailRepeatingTheQuestionIsDropped() {
    var ask = NeedsYouItem(
        id: "ask:a", kind: .ask, rank: 1, since: .now, question: "Allow touch x")
    ask.detail = "touch x"
    #expect(ask.distinctDetail == nil)
    ask.detail = "rm -rf build"
    #expect(ask.distinctDetail == "rm -rf build")
    ask.detail = ""
    #expect(ask.distinctDetail == nil)
}

/// The owner's order (Oct 5): ask, block, decision, PR thread, PR review, then
/// a local review, on the daemon's tiers; and a kind this build doesn't know
/// still sorts after every one it does.
@Test("The pull-request kinds sort between a decision and a local review")
func thePullRequestKindsSortBetweenADecisionAndALocalReview() {
    let tier: UInt32 = 100_000_000
    func item(_ id: String, _ kind: NeedsYouKind, _ tiers: UInt32) -> NeedsYouItem {
        NeedsYouItem(id: id, kind: kind, rank: tiers * tier + 5, since: nil, question: id)
    }
    let merged = NeedsYou.merge([
        "a": [
            item("review", .review, 5), item("pr-review", .prReview, 4), item("decision", .decision, 2),
        ],
        "b": [item("pr-thread", .prThread, 3), item("ask", .ask, 0), item("blocked", .blocked, 1)],
    ])
    #expect(merged.map(\.itemID) == ["ask", "blocked", "decision", "pr-thread", "pr-review", "review"])
}

@Test("The pull-request kinds decode by their wire words, and an item may carry a link")
func thePullRequestKindsDecodeByTheirWireWords() throws {
    let json = Data(
        """
        {"items": [{"id": "pr-review:1", "kind": "pr_review", "also": ["pr_thread"], "rank": 1,
          "since": null, "workspace_id": null, "workspace_name": "", "repository_id": null,
          "task": null, "terminal": null, "worktree": null, "question": "Fix it", "detail": null,
          "ask_id": null, "link": "https://github.com/a/b/pull/1", "actions": []},
          {"id": "old:2", "kind": "ask", "rank": 2, "question": "no link key at all"}]}
        """.utf8)
    let list = try JSONDecoder().decode(NeedsYouList.self, from: json)
    #expect(list.items.map(\.kind) == [.prReview, .ask])
    #expect(list.items[0].also == [.prThread])
    #expect(list.items[0].link == "https://github.com/a/b/pull/1")
    #expect(list.items[1].link == nil, "a runner older than the field sends none")
}

/// The same pull request on two runners signed in as one login is one thing to
/// do (ov-315): it shows once, from the runner that has waited longest.
@Test("A review request on two runners shows once, from the longest-waiting")
func aReviewRequestOnTwoRunnersShowsOnce() {
    func request(_ rank: UInt32) -> NeedsYouItem {
        NeedsYouItem(id: "pr-review:0123456789abcdef", kind: .prReview, rank: rank, since: nil, question: "Fix it")
    }
    let other = NeedsYouItem(id: "pr-review:fedcba9876543210", kind: .prReview, rank: 700, since: nil, question: "Other")
    let merged = NeedsYou.merge([
        "studio": [request(500_000_000), other],
        "build-box": [request(400_000_000)],
    ])
    #expect(merged.map(\.itemID) == ["pr-review:fedcba9876543210", "pr-review:0123456789abcdef"])
    #expect(merged.last?.runner == "build-box", "the lowest rank is kept")
    // Only a request is deduplicated: the same id on two runners elsewhere is two items.
    let ask = { NeedsYouItem(id: "ask:x", kind: .ask, rank: 5, since: nil, question: "") }
    #expect(NeedsYou.merge(["a": [ask()], "b": [ask()]]).count == 2)
}

@Test("A link opens only on github.com, bare, over https")
func aLinkOpensOnlyOnGitHub() {
    #expect(GitHubLink.url("https://github.com/acme/billing/pull/1#discussion_r1")?.host == "github.com")
    #expect(GitHubLink.url("https://GitHub.com/a/b/pull/1") != nil)
    for bad in [
        nil, "", "http://github.com/a", "https://github.com.evil.example/a", "https://github.com@evil.example/a",
        "https://evilgithub.com/a", "https://github.com:8443/a", "https://github.com/a b",
        "https://evil.example\\@github.com/a", "https://gitlab.com/a", "javascript:alert(1)",
        "https://user:pw@github.com/a",
    ] {
        #expect(GitHubLink.url(bad) == nil, "\(bad ?? "nil")")
    }
}

@Test("A decision about a thread counts as a decision, also")
func aThreadCountsAsADecisionEvenAsAnAlso() {
    #expect(NeedsYouKind.prThread.isDecision && NeedsYouKind.decision.isDecision)
    #expect(!NeedsYouKind.prReview.isDecision && !NeedsYouKind.review.isDecision)
    let ask = NeedsYouItem(
        id: "ask:a", kind: .ask, also: [.prThread], rank: 1, since: nil, workspaceID: "w", question: "")
    let workspace = WorkspaceSummary(id: "w", name: "W", taskPrefix: "w", isMain: false, ordinal: 0)
    #expect(RunnerBoards.decisions(for: workspace, in: [ask]) == 1)
}
