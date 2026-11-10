import Foundation

public enum AgentConversation {
    /// "0:04", "1:12", "2:03:09": a timer's format, for spans between two times the row can't say (a group of tools).
    public static func short(ms: Int64) -> String {
        let seconds = max(0, ms / 1000)
        let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60)
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }
}
