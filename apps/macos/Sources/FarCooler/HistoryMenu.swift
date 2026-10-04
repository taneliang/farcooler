import AgentKit
import SwiftUI

/// One row of the history a long press on Back or Forward opens (ov-248),
/// and of Workspace ▸ History: a place, the way the jump bar and the
/// switcher draw one.
struct PlaceRow: Equatable, Identifiable {
    var spot: NavigationHistory.Spot
    var title: String
    /// Quiet, after the title: the workspace it's in, and its runner when
    /// the window talks to more than one.
    var subtitle: String?
    var symbol: String
    /// The row as a menu item says it: the title, then where it is.
    var line: String { subtitle.map { "\(title) · \($0)" } ?? title }

    /// The place the window is at: checked.
    var current: Bool { spot == .current }

    var id: String {
        switch spot {
        case .forward(let n): "forward-\(n)"
        case .current: "current"
        case .back(let n): "back-\(n)"
        }
    }
}

enum HistoryMenu {
    typealias Selection = ContentView.Selection

    /// What the window knows of a place's names, each nil when it can't say
    /// (a runner not yet read, a place that's gone).
    struct Names {
        var workspace: (_ host: String, _ id: String) -> String?
        var task: (_ host: String, _ workspace: String, _ id: String) -> String?
        var worktree: (_ host: String, _ id: String) -> String?
    }

    /// `rows`, named and drawn: the symbol of its kind, its last crumb as the
    /// title, and where it is as the subtitle.
    static func rows(_ rows: [NavigationHistory.Row], names: Names, showHosts: Bool) -> [PlaceRow] {
        rows.map { row in
            let (title, symbol, workspace, host) = describe(row.stop.place, names: names)
            let subtitle = [workspace, showHosts && !host.isEmpty ? host : nil].compactMap { $0 }.joined(separator: " · ")
            return PlaceRow(
                spot: row.spot, title: shortened(title), subtitle: subtitle.isEmpty ? nil : subtitle, symbol: symbol)
        }
    }

    /// How much of a title a row keeps: a menu has no room for a task's whole
    /// sentence.
    static let titleLimit = 48

    static func shortened(_ title: String) -> String {
        title.count > titleLimit ? title.prefix(titleLimit - 1).trimmingCharacters(in: .whitespaces) + "…" : title
    }

    private static func describe(_ place: Selection, names: Names) -> (title: String, symbol: String, workspace: String?, host: String) {
        switch place {
        case .needsYou:
            return ("Needs You", "tray", nil, "")
        case .workspace(let host, let id, nil):
            return (names.workspace(host, id) ?? "Workspace", "square.stack.3d.up", nil, host)
        case .workspace(let host, let id, .task(let task)?):
            return (names.task(host, id, task) ?? "Task", "checklist", names.workspace(host, id), host)
        case .workspace(let host, let id, .worktree(let worktree, _)?):
            return (names.worktree(host, worktree) ?? "Worktree", "arrow.triangle.branch", names.workspace(host, id), host)
        case .workspace(let host, let id, .history(let status)?):
            return ("\(status.title) History", "clock.arrow.circlepath", names.workspace(host, id), host)
        case .looseWorktree(let host, let worktree, _):
            return (names.worktree(host, worktree) ?? "Worktree", "arrow.triangle.branch", nil, host)
        }
    }
}

/// The rows as a menu's items, for the title bar's buttons and the menu
/// bar's submenu alike: the place the window is at carries the checkmark;
/// choosing another goes there in one move.
struct HistoryMenuItems: View {
    let rows: [PlaceRow]
    let go: (NavigationHistory.Spot) -> Void

    var body: some View {
        ForEach(rows) { row in
            Toggle(isOn: Binding(get: { row.current }, set: { _ in if !row.current { go(row.spot) } })) {
                // One line: a menu item drawn from SwiftUI has no subtitle.
                Label(row.line, systemImage: row.symbol)
            }
        }
    }
}
