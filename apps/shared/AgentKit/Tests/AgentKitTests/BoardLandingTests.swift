import Foundation
import Testing

@testable import AgentKit

// Whether a board card's Agent button (or a deep link) has somewhere to land.
// `ShellScreen` asks exactly this before closing the board: a nil here is the
// board saying "That agent’s pane has closed." instead of closing onto
// nothing.

private func tab(_ id: String) -> ShellTab {
    ShellTab(id: id, title: id, mark: GlanceMark(attention: .quiet, core: .producing))
}

private let fleet = ShellFleet(workspaces: [
    ShellWorkspace(id: "w1", name: "one", tabs: [tab("w1-changes"), tab("w1-t1")]),
    ShellWorkspace(id: "w2", name: "two", isHidden: true, tabs: [tab("w2-changes"), tab("w2-t1")]),
])

/// A terminal whose tab the fleet has lands on it, hidden workspace or not:
/// the shell draws hidden workspaces too (the overview's Hidden section), so
/// "hidden" is not a place a board jump can fail to reach.
@Test func aTerminalWithATabLandsOnIt() {
    let tabs = ["t1": "w1-t1", "t2": "w2-t1"]
    #expect(fleet.landing(forTerminal: "t1", tabOfTerminal: tabs) == "w1-t1")
    #expect(fleet.landing(forTerminal: "t2", tabOfTerminal: tabs) == "w2-t1")
}

/// A terminal the map has never heard of has closed.
@Test func aTerminalNobodyKnowsHasNowhereToLand() {
    #expect(fleet.landing(forTerminal: "gone", tabOfTerminal: ["t1": "w1-t1"]) == nil)
}

/// A terminal the map composed a tab for, but whose tab this fleet does not
/// hold — the fleet moved on while the board was open — has nowhere to land
/// either, and must not be handed to the shell as if it did.
@Test func aTabTheFleetNoLongerHasIsNotALanding() {
    #expect(fleet.landing(forTerminal: "t9", tabOfTerminal: ["t9": "w9-t1"]) == nil)
}
