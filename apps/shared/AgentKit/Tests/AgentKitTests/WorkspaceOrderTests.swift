import Testing

@testable import AgentKit

/// The arithmetic behind dragging a worktree card.
///
/// Here rather than in a view test because there is no view test that could
/// catch any of this: a drop that lands one row short looks exactly like a drop
/// that lands right, in a screenshot and in a recording, and the difference only
/// shows up when somebody reaches for the card the next morning.
struct WorkspaceOrderTests {

    let list = ["a", "b", "c", "d"]

    @Test func droppingAboveARowPutsTheCardInFrontOfIt() {
        #expect(
            WorkspaceOrder.moved(list, dragging: "d", to: "b", .above) == ["a", "d", "b", "c"])
    }

    @Test func droppingBelowARowPutsTheCardAfterIt() {
        #expect(
            WorkspaceOrder.moved(list, dragging: "a", to: "c", .below) == ["b", "c", "a", "d"])
    }

    /// Dragging downwards is where a hand-written reorder goes wrong: the
    /// target's index shifts when the dragged card is lifted out, and using the
    /// index from before that lands the card one row short of where it was
    /// dropped — every time, in the same direction.
    @Test func draggingDownwardsLandsWhereItWasDroppedAndNotOneShort() {
        #expect(
            WorkspaceOrder.moved(list, dragging: "a", to: "d", .below) == ["b", "c", "d", "a"])
        #expect(
            WorkspaceOrder.moved(list, dragging: "a", to: "d", .above) == ["b", "c", "a", "d"])
        // And upwards, which is the direction that does not shift, so the two
        // must still agree about a card ending up at the very front.
        #expect(
            WorkspaceOrder.moved(list, dragging: "d", to: "a", .above) == ["d", "a", "b", "c"])
    }

    /// A move that changes nothing must come back identical, because that is how
    /// a caller decides not to spend a round trip — and a reorder request that
    /// changes nothing still makes every other client re-read the fleet.
    @Test func aDropThatWouldChangeNothingComesBackUnchanged() {
        // Onto itself.
        #expect(WorkspaceOrder.moved(list, dragging: "b", to: "b", .above) == list)
        // Onto the gap it already occupies, from both sides of it.
        #expect(WorkspaceOrder.moved(list, dragging: "b", to: "c", .above) == list)
        #expect(WorkspaceOrder.moved(list, dragging: "b", to: "a", .below) == list)
        // First card, dropped above the first card.
        #expect(WorkspaceOrder.moved(list, dragging: "a", to: "b", .above) == list)
    }

    /// A card the list does not hold cannot be placed by it. This is not
    /// theoretical: the fleet is re-read every few seconds, so a worktree can be
    /// removed by another client while somebody is mid-drag.
    @Test func aStrangerOnEitherEndLeavesTheListAlone() {
        #expect(WorkspaceOrder.moved(list, dragging: "z", to: "b", .above) == list)
        #expect(WorkspaceOrder.moved(list, dragging: "a", to: "z", .below) == list)
        #expect(WorkspaceOrder.moved([], dragging: "a", to: "b", .above) == [])
    }

    /// The midpoint, and nothing but the midpoint: a dead band through the
    /// middle of a row would make the gesture fail exactly where it is aimed
    /// most carefully, which is at the gap between two cards.
    @Test func theEdgeIsDecidedAtTheMidpointOfTheRow() {
        #expect(WorkspaceOrder.edge(pointerY: 0, rowHeight: 40) == .above)
        #expect(WorkspaceOrder.edge(pointerY: 19.9, rowHeight: 40) == .above)
        #expect(WorkspaceOrder.edge(pointerY: 20, rowHeight: 40) == .below)
        #expect(WorkspaceOrder.edge(pointerY: 40, rowHeight: 40) == .below)
        // A row that has not been measured yet must not fling a card to the end.
        #expect(WorkspaceOrder.edge(pointerY: 12, rowHeight: 0) == .above)
    }
}
