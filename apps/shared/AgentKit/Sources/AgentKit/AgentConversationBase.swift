import Foundation

public enum AgentConversation {
    /// "0:04", "1:12", "2:03:09": the core's timer format, for spans a row can't say (a group of tools); blank while the core can't answer.
    public static func short(ms: Int64) -> String {
        guard let answer = PlanViewPresenter.ask("{\"schema\":1,\"span\":\(ms)}"),
              let reply = try? JSONSerialization.jsonObject(with: Data(answer.utf8)) as? [String: Any] else { return "" }
        return reply["span"] as? String ?? ""
    }
}
