import Foundation
import Testing

/// The iOS harness's copy of the daemon's question is the real one, not a lookalike (ov-978).
struct HarnessWorkflowFixturesTests {
    @Test func theHarnessCopyIsTheProducersFixture() throws {
        let here = URL(fileURLWithPath: #filePath)
        let root = (0..<6).reduce(here) { url, _ in url.deletingLastPathComponent() }
        let fixture = try String(contentsOf: root.appendingPathComponent("crates/client/src/testdata/workflow-change-question.txt"), encoding: .utf8)
            .trimmingCharacters(in: .newlines)
        let harness = try String(contentsOf: root.appendingPathComponent("apps/ios/FarCooler/HarnessWorkflowFixtures.swift"), encoding: .utf8)
        #expect(harness.contains(fixture), "apps/ios/FarCooler/HarnessWorkflowFixtures.swift is stale: copy the fixture into it")
    }
}
