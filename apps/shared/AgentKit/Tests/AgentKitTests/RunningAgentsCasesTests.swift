import Foundation
import Testing

@testable import AgentKit

/// The agents at work, counted from the fleet the CLI writes, against
/// `test/fixtures/running-agents-cases.json`, which the Mac's
/// `WorkspaceAgentsTests` reads too (ov-660). The Mac decodes the fleet into
/// its own `Terminal` and counts with `MacAgents`; the phones decode it into
/// AgentKit's and count with `RunningAgents.workspace`. Same fleet, same
/// counts, or one of them has drifted: a card on the Mac and a card on an
/// iPhone beside the same plan would then disagree.
struct RunningAgentsCasesTests {
    static func fixture() throws -> [String: Any] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/running-agents-cases.json"))
        return try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    /// The fleet as one runner's connection holds it: only that runner's worktrees.
    static func fleet(host: String) throws -> Fleet {
        var fleet = try #require(try fixture()["fleet"] as? [String: Any])
        let worktrees = try #require(fleet["worktrees"] as? [[String: Any]])
        fleet["worktrees"] = worktrees.filter { ($0["host"] as? String ?? "") == host }
        return try JSONDecoder().decode(Fleet.self, from: JSONSerialization.data(withJSONObject: fleet))
    }

    @Test("AgentKit counts every workspace on every runner as the fixture says")
    func counts() throws {
        let cases = try #require(try Self.fixture()["cases"] as? [[String: Any]])
        #expect(cases.count >= 5)
        for c in cases {
            let workspace = try #require(c["workspace"] as? String)
            let host = try #require(c["host"] as? String)
            let expected = try #require(c["count"] as? Int)
            let fleet = try Self.fleet(host: host)
            let agents = RunningAgents.workspace(workspace, runner: "r", fleet: fleet)
            #expect(agents.count == expected, "\(workspace) on \(host.isEmpty ? "this runner" : host): \(c["why"] as? String ?? "")")
        }
    }
}
