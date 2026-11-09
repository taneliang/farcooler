import Foundation

/// The phrases the shell's close question, the Agents card and a card's answer row
/// share with the plan, asked of the Rust core (`crates/client/src/plan_view/words.rs`,
/// ov-1041, ruling R-112) through the bridge the plan view already uses. Title Case,
/// as the Mac and iPhone say it. Nil where no bridge is linked: a screen leaves the
/// phrase out rather than write it again.
public enum SharedWords {
    private struct Answer: Decodable {
        struct Words: Decodable {
            var agentsNone: String
            var waitingOnYou: String
            var blockedDoing: String

            enum CodingKeys: String, CodingKey {
                case agentsNone = "agents_none"
                case waitingOnYou = "waiting_on_you"
                case blockedDoing = "blocked_doing"
            }
        }
        var words: Words?
    }

    private static let cache = NSLock()
    nonisolated(unsafe) private static var asked: Answer.Words?

    /// Forget the answer and ask again: for a test that installs another bridge.
    public static func reset() {
        cache.lock()
        asked = nil
        cache.unlock()
    }

    private static var words: Answer.Words? {
        cache.lock()
        defer { cache.unlock() }
        if let asked { return asked }
        guard let answer = PlanViewPresenter.ask(#"{"schema":1,"words":{"style":"apple"}}"#),
              let decoded = try? JSONDecoder().decode(Answer.self, from: Data(answer.utf8)),
              let found = decoded.words
        else { return nil }
        asked = found
        return found
    }

    /// The Agents card's title when nothing is running.
    public static var agentsNone: String { words?.agentsNone ?? "" }
    /// The label of a card's answer row.
    public static var waitingOnYou: String { words?.waitingOnYou ?? "" }
    /// What a blocked agent is doing, in the middle of a sentence.
    public static var blockedDoing: String? { words?.blockedDoing }
}
