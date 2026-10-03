import AgentKit
import AppKit

// Moved out of TaskBoard.swift, unchanged, to keep it under its size ceiling
// when the navigator gained its Terminals section (ov-178).

/// The board list's keys (ov-85): ↑ and ↓ step through the tasks it shows,
/// top to bottom, opening each beside the board in place of the last.
enum BoardKeys {
    /// The tasks the list shows, top to bottom: the expanded sections' rows,
    /// as each is cut (`TaskBoardColumn.cut`): Done's by its rule, a long
    /// one's first ten until it shows more.
    /// `keeping`, the task selected, stays where it is though opening it
    /// read it, so ↑ and ↓ go on from it (ov-104 review).
    static func rows(
        _ board: TaskBoardModel, collapsed: Set<TaskStatus>, reads: BoardReads, keeping: String? = nil,
        showingMore: Set<TaskStatus> = [], filtering: Bool = false, now: Date
    ) -> [String] {
        // Filtering opens every section with a match.
        board.sections
            .filter { BoardForm.isExpanded($0, collapsed: filtering ? [] : collapsed) }
            .flatMap {
                $0.cut(
                    reads: reads, keeping: keeping, showingAll: showingMore.contains($0.status), filtering: filtering,
                    now: now)
                    .rows.map(\.id)
            }
    }

    /// The task `by` rows on from `current` in `ids`, held at either end;
    /// from none, or one no longer shown, the first going down and the last
    /// going up. Nil with nothing shown.
    static func step(from current: String?, by: Int, in ids: [String]) -> String? {
        guard !ids.isEmpty else { return nil }
        guard let current, let at = ids.firstIndex(of: current) else { return by >= 0 ? ids.first : ids.last }
        return ids[min(max(at + by, 0), ids.count - 1)]
    }

    /// The step a key event asks for: −1 for a bare ↑, 1 for a bare ↓, nil
    /// for anything else. Arrow keys carry the function and keypad flags of
    /// their own, which don't count as modifiers.
    static func arrow(keyCode: UInt16, modifiers: NSEvent.ModifierFlags) -> Int? {
        let held = modifiers.intersection(.deviceIndependentFlagsMask).subtracting([.function, .numericPad])
        guard held.isEmpty else { return nil }
        switch keyCode {
        case 125: return 1
        case 126: return -1
        default: return nil
        }
    }

    static func row(_ id: String?, in board: TaskBoardModel) -> TaskRow? {
        guard let id else { return nil }
        return board.columns.lazy.flatMap(\.rows).first { $0.id == id }
    }
}
