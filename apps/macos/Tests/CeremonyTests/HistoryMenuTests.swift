import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// What each row of the history menu says (ov-248): the symbol of its kind,
/// its place's name, and, quietly, where it is.
struct HistoryMenuTests {
    private typealias Selection = ContentView.Selection

    private static var names: HistoryMenu.Names { HistoryMenu.Names(
        workspace: { $1 == "billing" ? "Billing" : nil },
        task: { $2 == "t1" ? "bil-9 Invoice PDF export" : nil },
        worktree: { $1 == "lane" ? "invoice-pdf" : nil }) }

    private static func rows(_ places: [Selection], showHosts: Bool = false) -> [PlaceRow] {
        var history = NavigationHistory()
        for (old, new) in zip(places, places.dropFirst()) { history.record(from: old, to: new) }
        let current = places.last
        return HistoryMenu.rows(
            history.rows(current: current, trail: nil) { _ in true }, names: names, showHosts: showHosts)
    }

    @Test("Each kind of place has its symbol, its name and where it is; the place the window is at is the checked one")
    func kinds() {
        let rows = Self.rows([
            .needsYou,
            .workspace(host: "", workspace: "billing", focus: nil),
            .workspace(host: "", workspace: "billing", focus: .task("t1")),
            .workspace(host: "", workspace: "billing", focus: .worktree("lane", terminal: nil)),
            .workspace(host: "", workspace: "billing", focus: .history(.cancelled)),
            .looseWorktree(host: "", worktree: "lane", terminal: nil),
        ])
        // Nearest first below the checked one.
        #expect(rows.map(\.title) == [
            "invoice-pdf", "Canceled History", "invoice-pdf", "bil-9 Invoice PDF export", "Billing", "Needs You",
        ])
        #expect(rows.map(\.symbol) == [
            "arrow.triangle.branch", "clock.arrow.circlepath", "arrow.triangle.branch", "checklist",
            "square.stack.3d.up", "tray",
        ])
        #expect(rows.map(\.subtitle) == [nil, "Billing", "Billing", "Billing", nil, nil])
        #expect(rows.map(\.current) == [true, false, false, false, false, false])
    }

    @Test("A name it can't say yet is the kind's own word, and a runner is named only when there are several")
    func unknownAndHosts() {
        let places: [Selection] = [
            .workspace(host: "studio", workspace: "gone", focus: nil),
            .workspace(host: "studio", workspace: "gone", focus: .task("t9")),
        ]
        #expect(Self.rows(places).map(\.title) == ["Task", "Workspace"])
        #expect(Self.rows(places).map(\.subtitle) == [nil, nil])
        #expect(Self.rows(places, showHosts: true).map(\.subtitle) == ["studio", "studio"])
    }

    @Test("Each row says where choosing it goes, and ids are distinct")
    func spots() {
        let rows = Self.rows((0..<5).map { .workspace(host: "", workspace: "billing", focus: .task("t\($0)")) })
        #expect(rows.map(\.spot) == [.current, .back(0), .back(1), .back(2), .back(3)])
        #expect(Set(rows.map(\.id)).count == rows.count)
    }
}
