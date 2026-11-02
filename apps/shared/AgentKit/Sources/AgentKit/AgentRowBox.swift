import Foundation
import Observation

@MainActor
@Observable
public final class AgentRowBox: Identifiable {
    public let id: String
    public internal(set) var row: AgentRow {
        didSet {
            isTurn = Self.isTurn(row)
            let now = AgentTray.Activity(row)
            if now != trayActivity { trayActivity = now }
        }
    }
    /// Whether the row is a turn, read without observing the row (ov-691):
    /// what finds the newest turn reads this of every row after it, and
    /// observing those rows redrew whatever asked, the whole transcript
    /// and the composer, on every streamed word.
    @ObservationIgnored public private(set) var isTurn: Bool
    /// What the agents tray reads of the row (ov-693): a call running,
    /// thinking not yet ended, or neither. Observed apart from `row`, and set
    /// only when it changes, so the tray hears a call start or end and not
    /// each word streamed into a reply.
    public private(set) var trayActivity: AgentTray.Activity

    init(_ row: AgentRow) {
        id = row.id
        self.row = row
        isTurn = Self.isTurn(row)
        trayActivity = AgentTray.Activity(row)
    }

    private static func isTurn(_ row: AgentRow) -> Bool {
        if case .turn = row.kind { return true }
        return false
    }
}
