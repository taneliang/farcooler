import Testing

@testable import Far_Cooler

/// The runners' trouble in the toolbar (ov-105): hidden while every runner
/// is well and current, a few words and a menu while one isn't.
struct RunnerStatusItemTests {
    private typealias Item = RunnerStatusItem

    @Test("Nothing at all while every runner is well and current")
    func healthyIsHidden() {
        #expect(Item.label(troubles: [], stale: []) == nil)
        #expect(Item.entries(troubles: [], stale: []).isEmpty)
    }

    @Test("One runner offline: its name, Reconnect it, and Runners and Devices…")
    func oneOffline() {
        let troubles = [Item.Trouble(host: "carl", problem: .offline)]
        #expect(Item.label(troubles: troubles, stale: []) == "carl offline")
        #expect(Item.symbol(troubles: troubles) == "exclamationmark.triangle")
        let entries = Item.entries(troubles: troubles, stale: [])
        #expect(entries.map(\.title).filter { !$0.isEmpty } == ["Reconnect carl", "Runners and Devices…"])
    }

    @Test("Two runners offline: counted, each with Reconnect, and Reconnect All")
    func twoOffline() {
        let troubles = [Item.Trouble(host: "carl", problem: .offline), Item.Trouble(host: "", problem: .offline)]
        #expect(Item.label(troubles: troubles, stale: []) == "2 runners offline")
        let entries = Item.entries(troubles: troubles, stale: [])
        #expect(entries.contains(.reconnect(host: "carl")))
        #expect(entries.contains(.reconnect(host: "")))
        #expect(entries.contains(.reconnectAll))
        #expect(Item.Entry.reconnect(host: "").title == "Reconnect This Mac")
    }

    @Test("A runner without tmux says so, not that it's offline")
    func degraded() {
        #expect(Item.problem(.connected) == .noTmux)
        #expect(Item.problem(.reconnecting(attempt: 2)) == .offline)
        #expect(Item.problem(.connecting) == nil)
        #expect(Item.label(troubles: [.init(host: "carl", problem: .noTmux)], stale: []) == "tmux unavailable on carl")
    }

    @Test("A runner behind this app's build shows the item, with the update in its menu")
    func staleShowsTheUpdate() {
        #expect(Item.label(troubles: [], stale: ["carl"]) == "Update available")
        #expect(Item.symbol(troubles: []) == "arrow.down.circle")
        let entries = Item.entries(troubles: [], stale: ["carl"])
        #expect(entries.contains(.update(count: 1)))
        #expect(entries.contains(.note("carl isn’t running this app’s build")))
        #expect(Item.Entry.update(count: 2).title == "Update 2 Runners…")
    }

    /// A runner newer than this Mac (ov-143) lost its home with the old
    /// sidebar's dot and card (ov-178). The runner item says so, quietly,
    /// with what to do, and never offers an update: one from here would
    /// install this Mac's older build over it.
    @Test("A runner ahead of this Mac shows a quiet mark, says to update this Mac, and offers no update")
    func aheadIsSaidWithNoUpdate() {
        #expect(Item.label(troubles: [], stale: [], ahead: ["carl"]) == "Runner is newer")
        #expect(Item.symbol(troubles: [], stale: [], ahead: ["carl"]) == "info.circle")
        #expect(
            Item.help(troubles: [], stale: [], ahead: ["carl"])
                == "carl is newer than this Mac. Update Far Cooler on this Mac.")
        let entries = Item.entries(troubles: [], stale: [], ahead: ["carl"])
        #expect(entries.contains(.note("carl is newer than this Mac. Update Far Cooler on this Mac.")))
        #expect(!entries.contains { if case .update = $0 { true } else { false } })
        // Beside a runner that is behind, the update is that one's alone.
        let both = Item.entries(troubles: [], stale: ["box"], ahead: ["carl"])
        #expect(both.contains(.update(count: 1)))
        #expect(Item.label(troubles: [], stale: [], ahead: ["carl", ""]) == "2 runners are newer")
    }
}
