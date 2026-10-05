import AgentKit
import AppKit
import SwiftUI

/// The navigator's one task row (ov-104, the owner's pick of three, variant
/// B): the key in monospace and the title on one line, and a quiet second
/// line under the title. No card: the navigator is a list now, not a
/// kanban. The task list's rows, Unread's items and Activity's entries, and
/// the History page's rows are all this, so every task reads the same
/// wherever it's listed.
///
/// The key in a column of its own (`TaskKeyColumn`), so every title on a
/// board starts at one x.
struct CompactTaskRow<Second: View>: View {
    let key: String
    let title: String
    /// Needs Decision: the title semibold.
    var emphasized = false
    var selected = false
    /// The list has the keyboard: selected reads in the accent.
    var keyed = false
    /// Just arrived: a brief accent wash, faded by the caller (ov-104).
    var highlighted = false
    /// The rows its key and title are reported on for `GridGeometryTests`.
    var keyMark = "card"
    var titleMark: String?
    @ViewBuilder let second: () -> Second

    @Environment(\.taskKeyWidth) private var keyWidth

    @ViewBuilder private var titleText: some View {
        let text = Text(title)
            .font(.system(size: WorkspaceStyle.PaneText.body, weight: emphasized ? .semibold : .regular))
            // Two lines before the ellipsis (ov-177, the owner: "task names
            // can probably be truncated only after 2 lines"), its height its
            // own, so a row moving in a list keeps the height it lands with.
            .lineLimit(2)
            .truncationMode(.tail)
            .fixedSize(horizontal: false, vertical: true)
        if let titleMark { text.gridMark(titleMark, .text) } else { text }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Text(key)
                    .font(TaskKeyColumn.font)
                    .foregroundStyle(SidebarInk.secondary)
                    .lineLimit(1)
                    .frame(width: keyWidth, alignment: .leading)
                    .gridMark(keyMark, .text)
                    // What the task is, on hover (ov-299); its title is
                    // beside it, so VoiceOver's label stays the key.
                    .taskKeyCard(key, speaksTitle: false)
                titleText
                Spacer(minLength: 0)
            }
            HStack(alignment: .top, spacing: 0) {
                Color.clear.frame(width: keyWidth, height: 1)
                second()
                Spacer(minLength: 0)
            }
        }
        .help(title)
        .navigatorRow(selected: selected, keyed: keyed, box: keyMark)
        .background {
            RoundedRectangle.control
                .fill(highlighted ? Fill.selection(active: true) : Color.clear)
                .boxOutset()
        }
    }
}

/// The key column every compact row shares: as wide as the board's widest
/// key, and a gap.
@MainActor
enum TaskKeyColumn {
    static let font = Font.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced)
    private static let measured = NSFont.monospacedSystemFont(ofSize: WorkspaceStyle.PaneText.secondary, weight: .regular)
    static let gap: CGFloat = 8

    static func width(for keys: some Sequence<String>) -> CGFloat {
        let widest = keys.map { ($0 as NSString).size(withAttributes: [.font: measured]).width }.max() ?? 0
        return (widest + gap).rounded(.up)
    }
}

private struct TaskKeyWidthKey: EnvironmentKey {
    /// "ov-12" and the gap, before a board sets its own.
    static let defaultValue: CGFloat = 40
}

extension EnvironmentValues {
    /// How wide the key column of a compact row is, set once for a board.
    var taskKeyWidth: CGFloat {
        get { self[TaskKeyWidthKey.self] }
        set { self[TaskKeyWidthKey.self] = newValue }
    }
}
