import Foundation
import Testing

@testable import AgentKit

/// The phrases the shell's close question, the Agents card and a card's answer row
/// share with the plan (ov-1041): the bridge this package's tests link answers
/// with what `test/fixtures/plan-words.json` pins (the Rust test's own), and the
/// screens say it.
struct SharedWordsTests {
    private static func golden() throws -> [String: String] {
        let data = try Data(contentsOf: PlanPhoneFixtures.root.appendingPathComponent("test/fixtures/plan-words.json"))
        let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let apple = try #require(object["apple"] as? [String: Any])
        return try #require(apple["words"] as? [String: String])
    }

    @Test("the core's words are the ones the apps showed before they asked")
    func theWordsAreTheCores() throws {
        SharedWords.reset()
        let golden = try Self.golden()
        #expect(SharedWords.agentsNone == golden["agents_none"])
        #expect(SharedWords.agentsNone == "No Agents Running")
        #expect(SharedWords.blockedDoing == golden["blocked_doing"])
        #expect(ShellClose.doing(activity: "blocked") == "waiting on you")
        #expect(ShellClose.doing(activity: "working") == "working")
    }
}
