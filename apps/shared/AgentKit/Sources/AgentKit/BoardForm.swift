import Foundation

/// How a board's list is drawn: the status sections, which open and close,
/// and what a quiet board says.
///
/// Every board is this list, on every device. The Mac drew a kanban too,
/// when its board was wide, and the owner removed it (ov-83): the board
/// almost always sits in a narrow column beside the conversation, where a
/// kanban showed fewer statuses than the list does.
public enum BoardForm {
    /// The statuses whose sections start collapsed: finished work, which is
    /// most of an old board and the part nobody came to look at.
    public static let collapsedByDefault: Set<TaskStatus> = [.done, .cancelled]

    /// Whether a section's header opens. An empty status is a header reading
    /// "Backlog 0" and nothing under it, so it can't.
    public static func canExpand(_ section: TaskBoardColumn) -> Bool { section.count > 0 }

    /// One line naming the statuses with no task, or nil when none is empty.
    ///
    /// Four headers each reading zero were most of a quiet board, drawn as dim
    /// rows nothing could be done with. They are said once, at the end.
    public static func emptyNote(_ board: TaskBoardModel) -> String? {
        let names = board.columns.filter { $0.count == 0 }.map(\.title)
        switch names.count {
        case 0: return nil
        case 1: return "\(names[0]) is empty."
        default:
            return names.dropLast().joined(separator: ", ") + " and \(names.last!) are empty."
        }
    }

    /// Whether a board has nothing on it at all: no task in any status, and no
    /// row this build can't place. Seven headers each reading zero is not a
    /// board, it's a blank page, so the list says "No Tasks" instead.
    public static func isBlank(_ board: TaskBoardModel) -> Bool {
        board.rows.isEmpty && board.unreadable.isEmpty
    }

    /// What an empty board says under "No Tasks" (ov-205). The orchestrator
    /// owns the task list (ov-184), so a board it leads says to tell it what
    /// you want done, and, with none running, to start it first. A board on a
    /// runner too old for workspaces has no orchestrator to name.
    public static func blankLine(ledByOrchestrator: Bool, orchestratorRunning: Bool = true) -> String {
        guard ledByOrchestrator else { return FirstRunCopy.Phone.boardImplicit }
        return orchestratorRunning
            ? FirstRunCopy.Phone.boardWithOrchestrator : FirstRunCopy.Phone.boardNoOrchestrator
    }

    /// Whether a blank board offers Show Orchestrator: it's led by one, and
    /// there isn't one running to tell.
    public static func offersOrchestrator(ledByOrchestrator: Bool, orchestratorRunning: Bool) -> Bool {
        ledByOrchestrator && !orchestratorRunning
    }

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
