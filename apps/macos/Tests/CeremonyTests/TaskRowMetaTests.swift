import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A task row's one quiet metadata line (ov-92, owner: "the blue checkmark is
/// very distracting"): "<status or time> · <progress>", colored only where
/// the row needs the person, and no progress on a finished row with every
/// line met.
struct TaskRowMetaTests {
    private static let now = Date(timeIntervalSince1970: 1_759_400_000)

    private static func row(
        _ status: TaskStatus, met: Int = 0, of total: Int = 0, blocked: Bool = false, daysAgo: Double = 6
    ) -> TaskRow {
        let then = now.addingTimeInterval(-daysAgo * 86_400)
        return TaskRow(
            id: "t", key: "ov-28", title: "Phones: board cards show a relative updated time", status: status,
            statusSince: then,
            labels: ["owner"],
            acceptance: (0..<total).map { TaskAcceptanceLine(id: "a\($0)", text: "line \($0)", met: $0 < met) },
            blockedBy: blocked ? [TaskBlockRef(key: "ov-27", reason: "")] : [], createdAt: then, updatedAt: then)
    }

    @Test("Done with every line met reads only its time, quietly")
    func doneAllMet() {
        let line = TaskRowMeta.line(Self.row(.done, met: 5, of: 5), at: Self.now)
        #expect(line.progress == nil)
        #expect(line.tone == .quiet)
        #expect(line.text == "Done 6d ago")
        // Canceled the same.
        #expect(TaskRowMeta.progress(Self.row(.cancelled, met: 2, of: 2)) == nil)
    }

    @Test("Open lines read as N of M in the same quiet style, Done included")
    func openLines() {
        let line = TaskRowMeta.line(Self.row(.done, met: 3, of: 5), at: Self.now)
        #expect(line.progress == "3 of 5" && line.tone == .quiet)
        #expect(line.text == "Done 6d ago · 3 of 5 met")
        #expect(TaskRowMeta.progress(Self.row(.inProgress, met: 5, of: 5)) == "5 of 5")
        #expect(TaskRowMeta.progress(Self.row(.inProgress)) == nil)
    }

    /// No status gets color but a waiting decision; a task blocked on
    /// another does, whatever its status, and says what it's waiting on.
    @Test("Only a decision or a block colors the row")
    func onlyAttentionIsColored() {
        for status in TaskStatus.allCases {
            let tone = TaskRowMeta.line(Self.row(status, met: 5, of: 5, daysAgo: 0.1), at: Self.now).tone
            #expect(tone == (status == .needsDecision ? .attention : .quiet), "\(status)")
        }
        let blocked = TaskRowMeta.line(Self.row(.todo, blocked: true), at: Self.now)
        #expect(blocked.tone == .attention && blocked.lead == "Waiting on ov-27")
        // A task that hasn't moved in a day says so in orange, never the
        // accent.
        let stale = TaskRowMeta.line(Self.row(.inProgress, daysAgo: 3), at: Self.now)
        #expect(stale.tone == .stale && stale.lead?.hasPrefix("Hasn’t moved") == true)
    }
}
