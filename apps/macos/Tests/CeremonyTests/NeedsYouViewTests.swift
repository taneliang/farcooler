import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// What a Needs You row offers and says (spec §2.5).
struct NeedsYouViewTests {
    private static func item(
        _ kind: NeedsYouKind, actions: [NeedsYouAction] = [], askID: String? = nil, task: Bool = true
    ) -> NeedsYouItem {
        NeedsYouItem(
            id: "\(kind.rawValue):x", kind: kind, rank: 1, since: nil, workspaceID: "ws", workspaceName: "Billing",
            repositoryID: "r",
            task: task ? NeedsYouTask(id: "t-9", key: "bil-9", title: "Invoice PDF export", status: "in_review") : nil,
            terminal: NeedsYouTerminal(
                id: "t", worktreeID: "w", label: "claude", role: "agent", paneMode: "terminal"),
            question: "q", askID: askID, actions: actions)
    }

    private static let allowDeny = [
        NeedsYouAction(id: "deny", title: "Deny", destructive: true, primary: false),
        NeedsYouAction(id: "allow", title: "Allow touch x", destructive: false, primary: true),
    ]

    /// Both refusals are one code, `resource-conflict`; the `what:` word
    /// tells them apart, and the row keeps the one line that fits, naming
    /// its agent.
    @Test("A not_held refusal says someone already answered, and not_delivered says try again")
    func aNotHeldRefusalSaysSomeoneAlreadyAnswered() {
        let ask = Self.item(.ask, actions: Self.allowDeny, askID: "hook-ask-1")
        #expect(
            NeedsYouRowModel.refused(.from("error: x\ncode: resource-conflict\nwhat: not_held"), item: ask)
                == .refused("Someone already answered this."))
        #expect(
            NeedsYouRowModel.refused(.from("error: x\ncode: resource-conflict\nwhat: not_delivered"), item: ask)
                == .refused("Couldn’t reach claude. Try again."))
        // And an ask's buttons are its own options, in its order.
        #expect(
            NeedsYouRowModel.buttons(for: ask, canAct: true).buttons == [
                .ask(option: "deny", title: "Deny", destructive: true, primary: false),
                .ask(option: "allow", title: "Allow touch x", destructive: false, primary: true),
            ])
    }

    /// The inbox opens a review; it never approves one (ruling 2), whatever
    /// the runner sends.
    @Test("A review row can be opened, not approved")
    func aReviewRowCanBeOpenedNotApproved() {
        let review = Self.item(
            .review, actions: [NeedsYouAction(id: "approve", title: "Approve", destructive: false, primary: true)])
        let offered = NeedsYouRowModel.buttons(for: review, canAct: true)
        #expect(offered.buttons == [.review])
        #expect(offered.more.isEmpty)
    }

    /// Below Control scope the runner sends no actions, and every row has
    /// only Open, as the board offers no writes there. A decision's options
    /// past the third go in a menu; one with none offers Answer….
    @Test("A read-only runner's items have only Open")
    func aReadOnlyRunnersItemsHaveOnlyOpen() {
        let options = ["Postgres", "SQLite", "Both", "Neither"].map {
            NeedsYouAction(id: $0, title: $0, destructive: false, primary: false)
        }
        for item in [
            Self.item(.ask, actions: Self.allowDeny, askID: "hook-ask-1"), Self.item(.decision, actions: options),
            Self.item(.review), Self.item(.blocked),
        ] {
            #expect(NeedsYouRowModel.buttons(for: item, canAct: false).buttons == [.open], "\(item.kind)")
        }
        let decision = NeedsYouRowModel.buttons(for: Self.item(.decision, actions: options), canAct: true)
        #expect(decision.buttons == [.decide("Postgres"), .decide("SQLite"), .decide("Both")])
        #expect(decision.more == [.decide("Neither")])
        #expect(NeedsYouRowModel.buttons(for: Self.item(.decision), canAct: true).buttons == [.answerTyped])
    }

    private static let pull = "https://github.com/acme/billing/pull/213"

    private static func pullRequest(link: String?, workspace: String = "") -> NeedsYouItem {
        NeedsYouItem(
            id: "pr-review:0123456789abcdef", kind: .prReview, rank: 1, since: nil, workspaceName: workspace,
            question: "Add refund endpoint", detail: "acme/billing #213 · bob", link: link)
    }

    /// A teammate's pull request has one thing to do with it: read it on
    /// GitHub. Below Control the runner sends no link, and a row with no link
    /// has no button, because Open would go nowhere.
    @Test("A review request offers Review on GitHub, or nothing")
    func aReviewRequestOffersReviewOnGitHubOrNothing() {
        let linked = Self.pullRequest(link: Self.pull)
        for canAct in [true, false] {
            #expect(NeedsYouRowModel.buttons(for: linked, canAct: canAct).buttons == [.githubReview(Self.pull)])
        }
        for none in [
            nil, "", "javascript:alert(1)", "http://github.com/a/b/pull/1", "file:///etc/passwd", "https://",
            "https://github.com.evil.example/a/b/pull/1", "https://github.com@evil.example/a/b/pull/1",
            "https://evilgithub.com/a/b/pull/1", "https://github.com:8443/a/b/pull/1",
            "https://gitlab.com/a/b/pull/1", "https://github.com/a b",
        ] {
            #expect(
                NeedsYouRowModel.buttons(for: Self.pullRequest(link: none), canAct: true).buttons.isEmpty,
                "\(none ?? "nil")")
        }
        #expect(NeedsYouNavigation.tab(for: linked) == nil)
    }

    /// An escalated thread is answered as a decision is, with a way to read
    /// the thread first; its options go in the same menu.
    @Test("A PR thread is answered like a decision, beside a link to the thread")
    func aPRThreadIsAnsweredLikeADecision() {
        let options = ["Retry", "Leave it", "Ask", "Skip"].map {
            NeedsYouAction(id: $0, title: $0, destructive: false, primary: false)
        }
        var thread = Self.item(.prThread, actions: options)
        thread.link = Self.pull
        let offered = NeedsYouRowModel.buttons(for: thread, canAct: true)
        #expect(offered.buttons == [.github(Self.pull), .decide("Retry"), .decide("Leave it"), .decide("Ask")])
        #expect(offered.more == [.decide("Skip")])
        #expect(NeedsYouNavigation.tab(for: thread) == .overview)

        var bare = Self.item(.prThread)
        bare.link = Self.pull
        #expect(NeedsYouRowModel.buttons(for: bare, canAct: true).buttons == [.github(Self.pull), .answerTyped])
        // Below Control: the runner sends no link and no actions.
        #expect(NeedsYouRowModel.buttons(for: Self.item(.prThread), canAct: false).buttons == [.open])
    }

    /// A request belongs to no workspace, and "Unclaimed" would say a thing
    /// is wrong with it.
    @Test("A review request is filed under Pull Request, not Unclaimed")
    func aReviewRequestIsFiledUnderPullRequest() {
        #expect(NeedsYouRowModel.group(Self.pullRequest(link: Self.pull)) == "Pull Request")
        #expect(NeedsYouRowModel.group(Self.pullRequest(link: Self.pull, workspace: "Billing")) == "Billing")
        #expect(NeedsYouRowModel.group(Self.item(.blocked)) == "Billing")
        var unclaimed = Self.item(.blocked)
        unclaimed.workspaceName = ""
        #expect(NeedsYouRowModel.group(unclaimed) == "Unclaimed")
    }

    /// A sent answer spins until its item leaves, and no longer than the
    /// timeout: then the row gets its buttons back with a line, rather than
    /// spinning forever on a change that never arrived.
    @Test("An answered row doesn't spin forever")
    func anAnsweredRowDoesntSpinForever() {
        #expect(NeedsYouRowModel.settleTimeout <= .seconds(15))
        guard case .refused(let line) = NeedsYouRowModel.afterTimeout(.sending) else {
            Issue.record("still spinning")
            return
        }
        #expect(line.contains("hasn’t cleared"))
        #expect(NeedsYouRowModel.afterTimeout(.idle) == .idle)
        #expect(NeedsYouRowModel.afterTimeout(.refused("x")) == .refused("x"))
    }
}
