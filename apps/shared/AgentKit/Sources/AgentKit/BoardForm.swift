import Foundation

/// Which form a board draws in: the status-sectioned list, or the kanban
/// (owner decision 3, spec §5).
///
/// **Chosen by the board's own width,** not the window's. The board shares
/// its window with a sidebar, the orchestrator's conversation and a task's
/// column, so a wide window is no promise of a wide board; the view measures
/// its own body and asks `resolve`.
///
/// The phones are always a list and don't ask.
public enum BoardForm: String, Sendable, Hashable {
    case list, kanban

    // The kanban's metrics, which the Mac's board draws with, so the width
    // at which three columns fit is derived from the columns it draws rather
    // than written down beside them.

    /// A kanban column's card width.
    public static let columnWidth: Double = 260
    /// The padding inside a column, each side.
    public static let columnPadding: Double = 10
    /// The gap between two columns.
    public static let columnSpacing: Double = 12
    /// The padding around the row of columns, each side.
    public static let boardPadding: Double = 14

    /// At this and up, a kanban: the width at which three whole columns fit,
    /// 3 × (260 + 2 × 10) + 2 × 12 + 2 × 14 = 892 pt. Narrower, a kanban
    /// shows fewer than three statuses, which the list does better.
    ///
    /// Spec §5 first said 824, which left out each column's own padding and
    /// took the outer padding as 10; the 2A measurement lane found three
    /// columns need 892 (`.claude/agent/reports/ui/2a-report.md`).
    public static let kanbanFrom: Double =
        3 * (columnWidth + 2 * columnPadding) + 2 * columnSpacing + 2 * boardPadding

    /// The hysteresis band: going up the board switches at `kanbanFrom`, and
    /// going down 24 pt lower, so a divider dragged back and forth across one
    /// line doesn't flicker the board between its forms.
    public static let hysteresis: Double = 24

    /// Below this, a list, whatever it was: 868 pt.
    public static let listBelow: Double = kanbanFrom - hysteresis

    /// The form for a board `width` points wide.
    ///
    /// - A forced form is the form, at any width.
    /// - Under Automatic, below `listBelow` a list and at `kanbanFrom` and up
    ///   a kanban. In between, the form it already had: going up it switches
    ///   at 892 and going down at 868. A board with no form yet is a list
    ///   there, since three columns don't fit.
    public static func resolve(width: Double, previous: BoardForm?, forced: Choice) -> BoardForm {
        switch forced {
        case .list: return .list
        case .kanban: return .kanban
        case .auto:
            if width < listBelow { return .list }
            if width >= kanbanFrom { return .kanban }
            return previous ?? .list
        }
    }

    /// Where a board's choice is kept on this device:
    /// `board.form.<host>.<workspace>`. Per device and not synced (ruling
    /// 7): a phone is always a list, so there's nothing to agree on.
    public static func key(host: String, workspace: String) -> String {
        "board.form.\(host).\(workspace)"
    }

    /// The person's choice in the board header's toggle: `≡` List, `▦`
    /// Kanban, or neither, which is Automatic.
    public enum Choice: String, CaseIterable, Sendable, Hashable {
        case auto, list, kanban

        /// The form this choice forces, or nil for Automatic.
        public var forced: BoardForm? {
            switch self {
            case .auto: nil
            case .list: .list
            case .kanban: .kanban
            }
        }

        /// The choice after tapping `form` in the toggle: that form, forced;
        /// or, when it's already the one forced, Automatic again.
        public func choosing(_ form: BoardForm) -> Choice {
            forced == form ? .auto : Choice(rawValue: form.rawValue) ?? .auto
        }

        /// This device's choice for one board. Automatic when none was made,
        /// and for a value this build can't read.
        public static func read(
            host: String, workspace: String, from defaults: UserDefaults = .standard
        ) -> Choice {
            defaults.string(forKey: BoardForm.key(host: host, workspace: workspace))
                .flatMap(Choice.init(rawValue:)) ?? .auto
        }

        /// Keep this choice for one board. Automatic is the default, so
        /// choosing it clears the slot rather than filling it.
        public func write(
            host: String, workspace: String, in defaults: UserDefaults = .standard
        ) {
            let key = BoardForm.key(host: host, workspace: workspace)
            if self == .auto {
                defaults.removeObject(forKey: key)
            } else {
                defaults.set(rawValue, forKey: key)
            }
        }
    }

    // MARK: - The list form's sections

    /// The statuses whose sections start collapsed: finished work, which is
    /// most of an old board and the part nobody came to look at.
    public static let collapsedByDefault: Set<TaskStatus> = [.done, .cancelled]

    /// Whether a section's header opens. An empty status is a header reading
    /// "Backlog 0" and nothing under it, so it can't.
    public static func canExpand(_ section: TaskBoardColumn) -> Bool { section.count > 0 }

    /// Whether a section is drawn open, given the statuses this board has
    /// collapsed. An empty one never is.
    public static func isExpanded(_ section: TaskBoardColumn, collapsed: Set<TaskStatus>) -> Bool {
        canExpand(section) && !collapsed.contains(section.status)
    }

    /// Where a board's collapsed sections are kept on this device:
    /// `board.collapsed.<host>.<workspace>`, beside its form.
    public static func collapsedKey(host: String, workspace: String) -> String {
        "board.collapsed.\(host).\(workspace)"
    }

    /// The sections this board has collapsed on this device:
    /// `collapsedByDefault` until the person opens or closes one. A status
    /// word this build doesn't know is dropped.
    public static func collapsed(
        host: String, workspace: String, from defaults: UserDefaults = .standard
    ) -> Set<TaskStatus> {
        guard let words = defaults.stringArray(forKey: collapsedKey(host: host, workspace: workspace))
        else { return collapsedByDefault }
        return Set(words.compactMap(TaskStatus.init(rawValue:)))
    }

    /// Keep this board's collapsed sections. An empty set is kept as one,
    /// so a board with Done opened on purpose stays that way.
    public static func setCollapsed(
        _ statuses: Set<TaskStatus>, host: String, workspace: String,
        in defaults: UserDefaults = .standard
    ) {
        defaults.set(
            statuses.map(\.rawValue).sorted(), forKey: collapsedKey(host: host, workspace: workspace))
    }
}
