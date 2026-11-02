import Foundation

public enum AgentConversation {
    public static func short(ms: Int64) -> String {
        let seconds = max(0, ms / 1000)
        let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60)
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }
}
