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
                id: "t", worktreeID: "w", label: "claude", role: "agent", paneMode: "terminal", chatCapable: true),
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
