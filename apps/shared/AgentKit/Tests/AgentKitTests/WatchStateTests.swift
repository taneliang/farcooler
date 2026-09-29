import Foundation
import Testing

@testable import AgentKit

/// The spec's rule about the watch, which nothing else can check.
///
/// "The three reachability states are distinguishable, and actions are disabled
/// in two of them" is a verification bullet with no natural home: the code that
/// obeys it draws screens, and screens are in a watchOS app target `swift test`
/// cannot build. So the decision itself was moved to `WatchState.resolve` and
/// this is what stands behind it.
///
/// The tests that matter most here are the two about AGE. They look redundant —
/// `resolve` cannot see a date, so of course it ignores one — and that is
/// exactly what they are defending. The tempting change to this file is a
/// staleness parameter, on the reasoning that a fresh snapshot is good enough
/// to act on. It is not: a fresh snapshot with an unreachable phone still means
/// the Allow button goes nowhere, and the person walks away believing they
/// answered.
struct WatchStateTests {
    private func snapshot(
        agents: [FleetSnapshot.Agent] = [], capturedAt: Date = Date()
    ) -> FleetSnapshot {
        FleetSnapshot(agents: agents, capturedAt: capturedAt, complete: true)
    }

    private var blockedAgent: FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: "t1", label: "Terminal 1", machine: "studio", status: "blocked",
            glyph: "?", headline: "Wants to run cargo test", line: "cargo test --workspace",
            feed: [], rank: 0, turnFailed: false, activityChangedAt: nil)
    }

    // MARK: - The three states are distinguishable

    @Test func aReachablePhoneWithASnapshotIsLive() {
        let fleet = snapshot(agents: [blockedAgent])
        #expect(WatchState.resolve(snapshot: fleet, reachable: true) == .live(fleet))
    }

    @Test func anUnreachablePhoneWithASnapshotIsCached() {
        let fleet = snapshot(agents: [blockedAgent])
        #expect(WatchState.resolve(snapshot: fleet, reachable: false) == .cached(fleet))
    }

    @Test func noSnapshotIsNothingWhicheverWayTheLinkIs() {
        #expect(WatchState.resolve(snapshot: nil, reachable: true) == .nothing)
        #expect(WatchState.resolve(snapshot: nil, reachable: false) == .nothing)
    }

    /// Reachable and empty-handed is still `nothing`, not a live empty fleet.
    ///
    /// The two read the same on a screen and are not the same thing: one is "no
    /// agents are running" and the other is "this watch has never been told".
    /// Only a snapshot can say the first.
    @Test func aReachablePhoneThatHasSaidNothingYetIsStillNothing() {
        #expect(WatchState.resolve(snapshot: nil, reachable: true).snapshot == nil)
    }

    // MARK: - Actions are disabled in two of the three

    @Test func onlyLiveMayAct() {
        let fleet = snapshot(agents: [blockedAgent])
        #expect(WatchState.live(fleet).canAct)
        #expect(!WatchState.cached(fleet).canAct)
        #expect(!WatchState.nothing.canAct)
    }

    // MARK: - Age decides nothing, in either direction

    @Test func aSnapshotFromASecondAgoIsStillCachedWhenThePhoneIsUnreachable() {
        let fresh = snapshot(agents: [blockedAgent], capturedAt: Date())
        let state = WatchState.resolve(snapshot: fresh, reachable: false)
        #expect(state == .cached(fresh))
        #expect(!state.canAct)
    }

    @Test func aSnapshotFromHoursAgoIsStillLiveWhenThePhoneIsReachable() {
        // Past `FleetSnapshot.staleAfter`, so every surface will render it with
        // reduced confidence — and it is still actionable, because the phone is
        // right there to carry the tap.
        let old = snapshot(
            agents: [blockedAgent],
            capturedAt: Date().addingTimeInterval(-FleetSnapshot.staleAfter * 3))
        let state = WatchState.resolve(snapshot: old, reachable: true)
        #expect(state == .live(old))
        #expect(state.canAct)
    }

    // MARK: - What the screens read

    @Test func bothStatesThatHoldAFleetHandItBack() {
        let fleet = snapshot(agents: [blockedAgent])
        #expect(WatchState.live(fleet).snapshot == fleet)
        #expect(WatchState.cached(fleet).snapshot == fleet)
        #expect(WatchState.nothing.snapshot == nil)
    }
}

/// The watch's Needs You section (ov-55 4C.2).
struct WatchListTests {
    private func agent(_ id: String, status: String, rank: UInt32) -> FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: id, label: "claude", machine: "studio", status: status, glyph: "", headline: id,
            line: "", feed: [], rank: rank, turnFailed: false, activityChangedAt: nil)
    }

    private let allow = NeedsYouAction(id: "allow", title: "Allow touch x", destructive: false, primary: true)
    private let deny = NeedsYouAction(id: "deny", title: "Deny", destructive: true, primary: false)
    private let open = NeedsYouAction(id: "open", title: "Open", destructive: false, primary: false)

    private func terminal(_ id: String) -> NeedsYouTerminal {
        NeedsYouTerminal(
            id: id, worktreeID: nil, label: "claude", role: "agent", paneMode: "terminal",
            chatCapable: true)
    }

    private func ask(actions: [NeedsYouAction], askID: String? = "hook-ask-1") -> NeedsYouItem {
        NeedsYouItem(
            id: "ask:hook-ask-1", kind: .ask, rank: 5, since: nil, workspaceName: "Billing",
            terminal: terminal("t1"), question: "Allow touch x", askID: askID, actions: actions,
            runner: "r1")
    }

    private func decision() -> NeedsYouItem {
        NeedsYouItem(
            id: "decision:7", kind: .decision, rank: 200_000_005, since: nil,
            workspaceName: "Billing",
            task: NeedsYouTask(id: "7", key: "bil-7", title: "Queue", status: "needs_decision"),
            question: "Postgres or SQLite?",
            actions: [NeedsYouAction(id: "Postgres", title: "Postgres", destructive: false, primary: false)],
            runner: "r1")
    }

    /// Items first, then the agents in the order they always had, even when
    /// an agent outranks every item.
    ///
    /// Mutation: `rows` as agents then items. Red.
    @Test("The watch lists items before agents")
    func theWatchListsItemsBeforeAgents() {
        let snapshot = FleetSnapshot(
            agents: [agent("t2", status: "working", rank: 0), agent("t1", status: "blocked", rank: 1)],
            capturedAt: Date(), complete: true,
            needsYou: [ask(actions: [allow, deny]), decision()])
        let rows = WatchList(snapshot).rows
        #expect(rows.map(\.id) == [
            "item:r1\u{1F}ask:hook-ask-1", "item:r1\u{1F}decision:7", "agent:t2", "agent:t1",
        ])
    }

    /// A snapshot from a phone that wrote no list has no items, and the
    /// agents as before.
    @Test func noListIsAgentsAsBefore() {
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked", rank: 1)], capturedAt: Date(), complete: true)
        #expect(WatchList(snapshot).items.isEmpty)
        #expect(WatchList(snapshot).rows.map(\.id) == ["agent:t1"])
    }

    /// The buttons are the ask's own options, in its order, and never `open`;
    /// each sends `terminal.agent_answer` for that option.
    ///
    /// Mutation: `.answer` built from a fixed Allow and Deny. Red: the
    /// titles differ.
    @Test("An ask's buttons on the watch are the item's actions")
    func anAsksButtonsOnTheWatchAreTheItemsActions() {
        let item = ask(actions: [deny, allow, open])
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked", rank: 1)], capturedAt: Date(), complete: true,
            needsYou: [item])
        #expect(WatchItemAction.of(item, in: snapshot) == .answer([deny, allow]))
        #expect(
            item.watchRequest(answering: allow)
                == .answer(terminal: "t1", request: "hook-ask-1", option: "allow"))
        #expect(item.watchRequest(answering: open) == nil)
    }

    /// Below Control scope an ask comes with no id and no options, so it has
    /// nothing to send: the watch opens its agent instead of drawing a button.
    ///
    /// Mutation: dropping the `askID` check. Red: `.answer`.
    @Test func anAskWithNothingToSendOpensItsAgent() {
        let item = ask(actions: [allow], askID: nil)
        let snapshot = FleetSnapshot(
            agents: [agent("t1", status: "blocked", rank: 1)], capturedAt: Date(), complete: true)
        #expect(WatchItemAction.of(item, in: snapshot) == .agent(terminal: "t1"))
        #expect(WatchItemAction.of(item, in: FleetSnapshot.empty) == .onPhone)
    }

    /// A decision's options are a board note, which the watch doesn't write.
    ///
    /// Mutation: `of` returning `.answer` for any item with actions. Red.
    @Test func aDecisionIsOpenedOnThePhone() {
        #expect(WatchItemAction.of(decision(), in: FleetSnapshot.empty) == .onPhone)
        #expect(decision().watchPlace == "Billing · bil-7")
    }
}

/// Derived items are hedged on the watch (ov-55 4C fix round 1).
struct WatchDerivedTests {
    /// Mutation: `watchHedge` always nil. Red.
    @Test func aDerivedItemIsHedgedOnTheWatchAndASentOneIsNot() {
        var item = NeedsYouItem(
            id: "blocked:t1", kind: .blocked, rank: 100_000_001, since: nil,
            question: "claude needs you", runner: "r1")
        #expect(item.watchHedge == nil)
        item.isDerived = true
        #expect(item.watchHedge == "Older runner: update it to see asks and decisions.")
    }
}
