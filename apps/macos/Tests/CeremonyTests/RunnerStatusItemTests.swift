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
}
