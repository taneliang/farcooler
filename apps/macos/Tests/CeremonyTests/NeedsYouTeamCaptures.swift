import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A member's Needs You on a runner with a team (ov-633): their own items
/// under Yours, a teammate's and the owner's under Everyone Else, each naming whose
/// it is. Long real names, so truncation shows.
enum NeedsYouTeamSample {
    static func item(
        _ id: String, _ kind: NeedsYouKind, rank: UInt32, minutes: Double, workspace: String,
        task: NeedsYouTask? = nil, label: String = "claude", question: String,
        actions: [NeedsYouAction] = [NeedsYouAction(id: "open", title: "Open", destructive: false, primary: false)],
        concern: NeedsYouConcern?
    ) -> NeedsYouItem {
        NeedsYouItem(
            id: id, kind: kind, rank: rank, since: Date().addingTimeInterval(-minutes * 60),
            workspaceID: "ws-\(workspace)", workspaceName: workspace, repositoryID: "repo", task: task,
            terminal: NeedsYouTerminal(id: "t-\(id)", worktreeID: nil, label: label, role: "agent", paneMode: "terminal"),
            question: question, askID: kind == .ask ? "hook-ask-\(id)" : nil, actions: actions, runner: "studio",
            concern: concern)
    }

    static let mine = NeedsYouConcern(handle: "alice", name: "Alice Chen-Montgomery", yours: true)
    static let bobs = NeedsYouConcern(handle: "bob", name: "Bartholomew Okafor-Lindqvist", yours: false)
    static let owners = NeedsYouConcern(handle: "", name: "", yours: false)

    static let items: [NeedsYouItem] = [
        item(
            "ask:a", .ask, rank: 99_999_000, minutes: 3, workspace: "Billing Platform",
            task: NeedsYouTask(id: "t1", key: "bil-42", title: "Invoice PDF export with per-region tax lines", status: "in_progress"),
            question: "Allow npm run test:integration -- --runInBand",
            actions: [
                NeedsYouAction(id: "allow", title: "Allow", destructive: false, primary: true),
                NeedsYouAction(id: "deny", title: "Deny", destructive: true, primary: false),
            ],
            concern: mine),
        item(
            "decision:b", .decision, rank: 299_998_000, minutes: 25, workspace: "Billing Platform",
            task: NeedsYouTask(id: "t2", key: "bil-51", title: "Retry failed webhooks with exponential backoff", status: "needs_decision"),
            question: "Postgres advisory locks or a Redis queue for the retry worker?",
            actions: [
                NeedsYouAction(id: "Postgres", title: "Postgres", destructive: false, primary: false),
                NeedsYouAction(id: "Redis", title: "Redis", destructive: false, primary: false),
            ],
            concern: nil),
        item(
            "ask:c", .ask, rank: 99_999_500, minutes: 1, workspace: "Mobile Checkout Redesign",
            task: NeedsYouTask(id: "t3", key: "mob-118", title: "Apple Pay sheet on the iPad split view", status: "in_progress"),
            question: "Allow xcodebuild -scheme Checkout -destination 'platform=iOS Simulator'",
            actions: [
                NeedsYouAction(id: "allow", title: "Allow", destructive: false, primary: true),
                NeedsYouAction(id: "deny", title: "Deny", destructive: true, primary: false),
            ],
            concern: bobs),
        item(
            "blocked:d", .blocked, rank: 199_990_000, minutes: 48, workspace: "Infrastructure",
            label: "codex", question: "Which AWS account should the staging bucket live in?",
            concern: owners),
    ]
}

@MainActor
@Suite(.serialized)
struct NeedsYouTeamTests {
    @Test("A split list draws both groups, and a solo list draws neither title")
    func aSplitListDrawsBothGroups() {
        let split = NeedsYou.grouped(NeedsYou.merge(["studio": NeedsYouTeamSample.items]))
        #expect(split.isSplit)
        #expect(split.yours.map(\.itemID) == ["ask:a", "decision:b"])
        #expect(split.others.map(\.othersName) == ["Bartholomew Okafor-Lindqvist", "Owner"])
        let solo = NeedsYouTeamSample.items.map { item -> NeedsYouItem in
            var item = item
            item.concern = nil
            return item
        }
        #expect(!NeedsYou.grouped(solo).isSplit)
    }

    @Test("Every row on a team's runner says who it is for, and a solo runner's says nothing (ov-737)")
    func everyRowSaysWhoseItIs() {
        let team = NeedsYou.teamRunners(in: NeedsYouTeamSample.items)
        #expect(
            NeedsYouTeamSample.items.map { $0.whose(teamRunners: team) }
                == ["You", "Anyone", "Bartholomew Okafor-Lindqvist", "Owner"])
        let solo = NeedsYouTeamSample.items.map { item -> NeedsYouItem in
            var item = item
            item.concern = nil
            return item
        }
        #expect(solo.allSatisfy { $0.whose(teamRunners: NeedsYou.teamRunners(in: solo)) == nil })
    }
}

/// Opt-in (FARCOOLER_CAPTURE_OUT): the page above, light and dark, at a
/// roomy width and a narrow one. The production view, drawn into a bitmap;
/// sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct NeedsYouTeamCaptures {
    @Test(arguments: [("wide", 760.0), ("narrow", 460.0)])
    func capture(width name: String, width: Double) throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let items = NeedsYou.merge(["studio": NeedsYouTeamSample.items])
        for (variant, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
            let page = NeedsYouView(
                items: items, olderRunners: [], canAct: { _ in true }, onOpen: { _ in },
                onAnswerAsk: { _, _ in nil }, onDecide: { _, _ in true }
            )
            .frame(width: width, height: 760)
            let host = NSHostingView(rootView: page)
            host.appearance = NSAppearance(named: appearance)
            host.frame = CGRect(x: 0, y: 0, width: width, height: 760)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.lookBitmap(scale: 2))
            try #require(rep.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("mac-needs-you-team-\(name)-\(variant).png"))
        }
    }
}
