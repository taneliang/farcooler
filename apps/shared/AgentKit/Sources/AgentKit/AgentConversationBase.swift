import Foundation

public enum AgentConversation {
    /// "0:04", "1:12", "2:03:09": the core's `conversation_words::short`.
    public static func short(ms: Int64) -> String {
        ConversationWords.item("ms", ms) as? String ?? "0:00"
    }
}
