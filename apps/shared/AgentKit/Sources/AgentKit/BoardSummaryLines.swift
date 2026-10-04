import Foundation

// The words a phone's Unread section says (ov-113), in the Mac's: its second
// lines, and what Mark All as Read asks.

extension BoardSummary.Item {
    /// "Done 2h ago", "Needs Decision 5m ago", "Added 3h ago".
    public func when(now: Date) -> String {
        let what = detail ?? (id.hasSuffix("/done") ? "Done" : "Added")
        return "\(what) \(TaskRow.ago(now.timeIntervalSince(at)))"
    }
}

extension BoardSummary.Activity {
    /// "12m ago · +2 more": when the newest note was written, and how many
    /// older ones the ticket has in the summary too.
    public func foot(now: Date) -> String {
        [TaskRow.ago(now.timeIntervalSince(at)), moreLine].compactMap { $0 }.joined(separator: " · ")
    }
}

extension BoardSummary {
    /// How many tasks the summary lists: one under Finished and Activity
    /// both counts once.
    public var taskCount: Int {
        Set((finished + moved + created).map(\.taskID) + activity.map(\.taskID)).count
    }

    /// What Mark All as Read says it will do. On a runner that keeps the
    /// state it clears every device, and the person is told so.
    public static func markAllReadMessage(tasks: Int, everywhere: Bool) -> String {
        let what = tasks == 1 ? "1 task will be marked as read" : "\(tasks) tasks will be marked as read"
        return everywhere ? "\(what) on all your devices." : "\(what)."
    }
}
