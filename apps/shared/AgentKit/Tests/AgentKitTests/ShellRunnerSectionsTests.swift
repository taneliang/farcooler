import Foundation
import Testing

@testable import AgentKit

/// The overview, one section per runner, and a drag inside one of them.
///
/// Everything a reorder decides is here rather than in the grid, for the reason
/// `ShellNavigationTests` gives: the iOS target has no unit tests, and the three
/// ways this goes wrong all look fine on a screenshot. A drop that sends one
/// runner's order to another runner reorders nothing and says nothing; a drop
/// that sends a SHORTER list than the section shows reorders around a card the
/// phone still thinks it moved; and a drop that is not applied until the runner
/// answers springs the card back to where it came from and then jumps it
/// forward again a round trip later.
struct ShellRunnerSectionsTests {
    private static func card(
        _ id: String, on runner: String?, hidden: Bool = false,
        loud: Bool = false
    ) -> ShellWorkspace {
        ShellWorkspace(
            id: id, name: id, isHidden: hidden, runner: runner,
            tabs: [
                ShellTab(
                    id: "\(id)/0", title: "claude",
                    mark: loud
                        ? GlanceMark(attention: .needsYou, core: .atAPrompt)
                        : GlanceMark(attention: .quiet, core: .producing),
                    wantsAttention: loud)
            ])
    }

    private static let laptop = ShellRunnerLabel(id: "L", name: "laptop", keepsOrder: true)
    private static let gpu = ShellRunnerLabel(id: "G", name: "gpu-box-2", keepsOrder: true)

    /// Two runners' worktrees, deliberately INTERLEAVED in the fleet.
    ///
    /// The real merge never interleaves — `ShellFleetMap.of` appends a runner
    /// at a time — and that is exactly why the fixture does: a grouping that
    /// only worked because the runners arrived in blocks would pass on the real
    /// merge and fail the first time anything reordered it.
    private static func fleet() -> ShellFleet {
        ShellFleet(workspaces: [
            card("l1", on: "L"),
            card("g1", on: "G"),
            card("l2", on: "L", loud: true),
            card("g2", on: "G"),
            card("l3", on: "L"),
        ])
    }

    private static func ids(_ section: ShellRunnerSection) -> [String] {
        section.cards.map(\.id)
    }

    // MARK: - Sections

    /// One section per runner, in the order the runners were given, holding
    /// that runner's worktrees and nobody else's.
    @Test func eachRunnerGetsASectionOfItsOwnWorktreesInRunnerOrder() {
        let sections = Self.fleet().runnerSections([Self.gpu, Self.laptop])
        #expect(sections.map(\.id) == ["G", "L"])
        #expect(Self.ids(sections[0]) == ["g1", "g2"])
        #expect(Self.ids(sections[1]) == ["l1", "l2", "l3"])
        #expect(
            sections[1].cards.map(\.index) == [0, 2, 4],
            "a card still names its place in the fleet, which is what its frame is keyed by")
    }

    /// **The order is the runner's, not precedence.** A card somebody put
    /// third stays third when its agent starts asking for them — otherwise a
    /// drag would be undone by the next agent that finishes, and the Mac and
    /// the phone would draw one runner's list in two different orders.
    @Test func aLoudWorktreeIsNotLiftedOutOfTheOrderSomebodyChose() {
        let laptop = Self.fleet().runnerSections([Self.laptop])[0]
        #expect(Self.ids(laptop) == ["l1", "l2", "l3"], "l2 needs you and still sits second")
    }

    /// Hidden worktrees are the Hidden section's, not their runner's.
    @Test func aHiddenWorktreeIsNotInItsRunnersSection() {
        var fleet = Self.fleet()
        fleet.workspaces[2].isHidden = true
        let laptop = fleet.runnerSections([Self.laptop])[0]
        #expect(Self.ids(laptop) == ["l1", "l3"])
        #expect(fleet.hiddenOrder() == [2])
    }

    /// **Every runner has a header while nothing is being searched for**,
    /// even one with nothing on it: the header is where that runner's Edit,
    /// Settings and Reconnect live now, and a runner with no worktrees is
    /// exactly the one somebody opens to correct.
    ///
    /// A search is different. A header over no cards reads as a runner that has
    /// gone empty, and what actually happened is that nothing on it matched.
    @Test func anEmptyRunnerHasAHeaderUntilASearchLeavesItWithNothing() {
        let empty = ShellRunnerLabel(id: "E", name: "fresh-box")
        let all = Self.fleet().runnerSections([Self.laptop, empty, Self.gpu])
        #expect(all.map(\.id) == ["L", "E", "G"])
        #expect(all[1].cards.isEmpty)

        let searched = Self.fleet().runnerSections([Self.laptop, empty, Self.gpu], matching: "g")
        #expect(searched.map(\.id) == ["G"], "only the runner with a match keeps its header")
        #expect(Self.ids(searched[0]) == ["g1", "g2"])
    }

    // MARK: - What a section allows

    @Test func aConnectedRunnerThatKeepsAnOrderCanBeReordered() {
        let laptop = Self.fleet().runnerSections([Self.laptop])[0]
        #expect(laptop.canReorder)
    }

    /// A search shows a subset, and a drop among a subset is a drop whose
    /// meaning depends on cards nobody can see.
    @Test func aSearchTurnsReorderingOff() {
        let laptop = Self.fleet().runnerSections([Self.laptop], matching: "l")[0]
        #expect(Self.ids(laptop) == ["l1", "l2", "l3"])
        #expect(!laptop.canReorder)
    }

    /// The write has nowhere to go, so the card must not pretend it went.
    @Test func aRunnerThatIsNotAnsweringCannotBeReordered() {
        let asleep = ShellRunnerLabel(
            id: "L", name: "laptop", isAnswering: false, keepsOrder: true)
        #expect(!Self.fleet().runnerSections([asleep])[0].canReorder)
    }

    /// A runner that does not keep an order — no `workspace_order` in its
    /// capabilities — draws a section with no drag in it. Which runners those
    /// are is `ShellRunnerLabel.keepsOrder(daemon:)`, tested below.
    @Test func aRunnerThatStoresNoOrderCannotBeReordered() {
        let old = ShellRunnerLabel(id: "L", name: "laptop", keepsOrder: false)
        #expect(!Self.fleet().runnerSections([old])[0].canReorder)
    }

    /// Two worktrees as `Session::fleet` puts them on the wire for a runner
    /// that predates `workspace_order`.
    ///
    /// **`ordinal` is PRESENT, and 0.** It is a proto3 scalar with no
    /// presence, prost decodes an old daemon's silence as 0, and
    /// `crates/client/src/session.rs` emits `"ordinal": w.ordinal`
    /// unconditionally — so "no ordinal" is not a thing the phone ever sees,
    /// and a rule that waited for one would offer every old runner a drag.
    private static func wire(ordinals: [Int]) throws -> [Workspace] {
        let rows = ordinals.enumerated().map { index, ordinal in
            """
            {"id": "w\(index)", "short": "w\(index)", "task": "t\(index)", "branch": "b\(index)",
             "state": "ready", "ordinal": \(ordinal), "terminals": []}
            """
        }
        return try JSONDecoder().decode(
            [Workspace].self, from: Data("[\(rows.joined(separator: ","))]".utf8))
    }

    /// **An old runner, all zeros and no `workspace_order`, is not offered a
    /// drag.** It would accept nothing — `workspace.reorder` is unknown to it —
    /// and the card would spring back with no error anywhere.
    @Test func aRunnerWithoutTheWorkspaceOrderCapabilityKeepsNoOrder() throws {
        let old = DaemonBuild(
            version: "0.1.0+old", matches: true, platform: "macos",
            capabilities: ["workspaces", "terminals", "watching"])
        let workspaces = try Self.wire(ordinals: [0, 0])
        #expect(
            workspaces.allSatisfy { $0.ordinal == 0 },
            "an old runner's ordinals arrive, as 0 — there is no absence to read")
        #expect(!ShellRunnerLabel.keepsOrder(daemon: old))
    }

    /// The capability is the whole answer, and a runner nobody has asked yet
    /// is refused until it has been asked rather than offered a drag on a
    /// guess.
    @Test func theWorkspaceOrderCapabilityIsWhatDecides() {
        let new = DaemonBuild(
            version: "0.1.0+new", matches: true, platform: "macos",
            capabilities: ["workspaces", "terminals", "workspace_order"])
        #expect(ShellRunnerLabel.keepsOrder(daemon: new))
        #expect(!ShellRunnerLabel.keepsOrder(daemon: nil))
        // A daemon so old it answered no capabilities at all is read as the
        // two features that existed then — which does not include this.
        let ancient = DaemonBuild(version: "0.0.1", matches: true, platform: "macos")
        #expect(!ShellRunnerLabel.keepsOrder(daemon: ancient))
    }

    @Test func aSectionOfOneHasNothingToReorder() {
        let fleet = ShellFleet(workspaces: [Self.card("a", on: "L")])
        #expect(!fleet.runnerSections([Self.laptop])[0].canReorder)
    }

    // MARK: - A drop

    /// The request carries the WHOLE section, first first, and only that
    /// runner's worktrees — the runner permutes exactly the rows it is named
    /// and leaves every other runner's alone, but each runner only has its own
    /// table to permute in.
    @Test func aDropSendsItsRunnerTheWholeSectionAndNothingElse() {
        let laptop = Self.fleet().runnerSections([Self.laptop, Self.gpu])[0]
        let request = laptop.reorder(moving: ["l3"], before: "l1")
        #expect(request == ShellReorderRequest(runner: "L", order: ["l3", "l1", "l2"]))
    }

    @Test func aDropAtTheEndMovesTheCardLast() {
        let laptop = Self.fleet().runnerSections([Self.laptop])[0]
        #expect(laptop.reorder(moving: ["l1"], before: nil)?.order == ["l2", "l3", "l1"])
    }

    /// Several cards dragged together keep the order they had between them.
    @Test func aDropOfSeveralCardsKeepsTheirOwnOrder() {
        let laptop = Self.fleet().runnerSections([Self.laptop])[0]
        #expect(laptop.reorder(moving: ["l3", "l1"], before: nil)?.order == ["l2", "l1", "l3"])
    }

    /// A drop that changes nothing costs no round trip: a reorder makes every
    /// other connected client re-read the fleet.
    @Test func aDropThatChangesNothingSendsNothing() {
        let laptop = Self.fleet().runnerSections([Self.laptop])[0]
        #expect(laptop.reorder(moving: ["l1"], before: "l2") == nil)
        #expect(laptop.reorder(moving: ["l3"], before: nil) == nil)
        #expect(laptop.reorder(moving: ["l2"], before: "l2") == nil)
    }

    /// A card carried into another runner's section is not a move either
    /// runner can make: a worktree lives on the machine its directory is on.
    @Test func aCardDroppedInAnotherRunnersSectionSendsNothing() {
        let sections = Self.fleet().runnerSections([Self.laptop, Self.gpu])
        #expect(sections[1].reorder(moving: ["l1"], before: "g1") == nil)
        #expect(sections[0].reorder(moving: ["g2"], before: nil) == nil)
        // And a drag of several cards that includes a foreign one: moving
        // just the local half would be a drop nobody made.
        #expect(sections[0].reorder(moving: ["l1", "g1"], before: nil) == nil)
    }

    @Test func aSectionThatCannotBeReorderedSendsNothing() {
        let searched = Self.fleet().runnerSections([Self.laptop], matching: "l")[0]
        #expect(searched.reorder(moving: ["l3"], before: "l1") == nil)
    }

    /// **All or nothing.** The shell's ids are resolved back to the daemon's
    /// own, and one that no longer resolves — a worktree removed between the
    /// lift and the drop — must not be quietly dropped from the list: the
    /// runner would reorder the rest around a card the phone still believes it
    /// moved. Nor may one resolve to a different runner.
    @Test func theRequestResolvesToThatRunnersOwnIdsOrToNothing() {
        let request = ShellReorderRequest(runner: "L", order: ["L/b", "L/a"])
        let table: [String: (runner: String, workspace: String)] = [
            "L/a": ("L", "uuid-a"), "L/b": ("L", "uuid-b"), "G/c": ("G", "uuid-c"),
        ]
        #expect(request.workspaceIDs { table[$0] } == ["uuid-b", "uuid-a"])

        let gone = ShellReorderRequest(runner: "L", order: ["L/b", "L/x", "L/a"])
        #expect(gone.workspaceIDs { table[$0] } == nil)

        let stray = ShellReorderRequest(runner: "L", order: ["L/b", "G/c"])
        #expect(stray.workspaceIDs { table[$0] } == nil)
    }

    // MARK: - Between the drop and the runner's answer

    /// The drop is drawn the moment it lands, not a round trip later.
    ///
    /// Not optimism about the outcome: the pending order is dropped the moment
    /// the call returns, whatever it returned, and by then the connection has
    /// already re-read the fleet — so what is on screen afterwards is the
    /// runner's answer, and a refusal puts the card back.
    @Test func aPendingOrderIsDrawnUntilItSettles() {
        let fleet = Self.fleet()
        var pending = ShellPendingOrders()
        let request = ShellReorderRequest(runner: "L", order: ["l3", "l1", "l2"])
        pending.begin(request)

        let during = fleet.runnerSections([Self.laptop, Self.gpu], pending: pending)
        #expect(Self.ids(during[0]) == ["l3", "l1", "l2"])
        #expect(Self.ids(during[1]) == ["g1", "g2"], "the other runner is not touched")
        #expect(
            during[0].cards.first?.index == 4,
            "a moved card still names its OWN place in the fleet")

        pending.settle(request)
        #expect(Self.ids(fleet.runnerSections([Self.laptop], pending: pending)[0]) == ["l1", "l2", "l3"])
    }

    /// A second drop before the first one's answer: the first answer arriving
    /// must not put the second drop back.
    @Test func anOlderAnswerDoesNotClearANewerDrop() {
        var pending = ShellPendingOrders()
        let first = ShellReorderRequest(runner: "L", order: ["l3", "l1", "l2"])
        let second = ShellReorderRequest(runner: "L", order: ["l2", "l3", "l1"])
        pending.begin(first)
        pending.begin(second)
        pending.settle(first)
        #expect(pending.order(for: "L") == ["l2", "l3", "l1"])
        pending.settle(second)
        #expect(pending.order(for: "L") == nil)
    }

    /// A poll while the drop is in flight can bring a worktree the drop never
    /// named, or take one away. The pending order permutes only the cards it
    /// named, among the places they hold — the runner's own rule — so a new
    /// card stays where the runner put it and a removed one simply goes.
    @Test func aPendingOrderLeavesCardsItDidNotNameWhereTheyAre() {
        var pending = ShellPendingOrders()
        pending.begin(ShellReorderRequest(runner: "L", order: ["l3", "l1", "l2"]))

        var grown = Self.fleet()
        grown.workspaces.insert(Self.card("new", on: "L"), at: 1)
        #expect(
            Self.ids(grown.runnerSections([Self.laptop], pending: pending)[0])
                == ["l3", "new", "l1", "l2"])

        var shrunk = Self.fleet()
        shrunk.workspaces.remove(at: 2)
        #expect(Self.ids(shrunk.runnerSections([Self.laptop], pending: pending)[0]) == ["l3", "l1"])
    }

    /// The same rule, applied to a whole fleet: the runner's order changes and
    /// every other runner's worktree keeps the exact slot it had. What the
    /// harness does in place of a runner, and what the daemon does in
    /// `Store::reorder_workspaces`.
    @Test func applyingARequestPermutesOnlyThatRunnersSlots() {
        let applied = Self.fleet().reordered(
            ShellReorderRequest(runner: "L", order: ["l3", "l1", "l2"]))
        #expect(applied.workspaces.map(\.id) == ["l3", "g1", "l1", "g2", "l2"])
    }
}

/// Which rests move the selected runner.
///
/// The selection decides where the NEXT launch lands, so a rest that follows
/// the wrong thing is a regression that outlives the process: one launch where
/// the selected runner was slow to answer seats the shell on another runner's
/// first worktree, and a rule that followed that landing would write it down
/// as the choice — every launch after it lands there too.
struct ShellSelectionTests {
    private func follows(_ arrival: ShellArrival, every: Bool = true) -> Bool {
        ShellSelection.follows(arrival, everyRunnerAtOnce: every, arrived: "B", selected: "A")
    }

    /// The launch landing is a race, not a choice.
    @Test func theLandingAtLaunchIsNotFollowed() {
        #expect(!follows(.appeared))
    }

    /// A worktree vanishing, or a runner answering, re-seats the shell;
    /// nobody chose where.
    @Test func aReseatIsNotFollowed() {
        #expect(!follows(.reseated))
    }

    /// A notification tapped is not a person choosing a runner to work on.
    @Test func aDeepLinkIsNotFollowed() {
        #expect(!follows(.linked))
    }

    /// A swipe or a tap onto another runner's worktree is.
    @Test func aMoveOntoAnotherRunnerIsFollowed() {
        #expect(follows(.moved))
        #expect(
            !ShellSelection.follows(
                .moved, everyRunnerAtOnce: true, arrived: "A", selected: "A"),
            "already selected: nothing to write")
    }

    /// With one runner connected at a time, the selection IS which runner is
    /// connected, and it only changes from a heading's Switch to This Runner.
    @Test func withOneRunnerAtATimeNothingIsFollowed() {
        #expect(!follows(.moved, every: false))
    }
}
