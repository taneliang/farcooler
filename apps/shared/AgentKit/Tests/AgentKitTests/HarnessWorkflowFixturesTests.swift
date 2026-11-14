import Foundation
import Testing

/// The iOS harness bundles the producer's fixture directly (ov-978).
struct HarnessWorkflowFixturesTests {
    @Test func theHarnessUsesTheProducersFixture() throws {
        let here = URL(fileURLWithPath: #filePath)
        let root = (0..<6).reduce(here) { url, _ in url.deletingLastPathComponent() }
        let fixture = try String(contentsOf: root.appendingPathComponent("crates/client/src/testdata/workflow-change-question.txt"), encoding: .utf8)
        let generator = try String(contentsOf: root.appendingPathComponent("apps/ios/generate-project.py"), encoding: .utf8)
        let harness = try String(contentsOf: root.appendingPathComponent("apps/ios/FarCooler/HarnessWorkflowFixtures.swift"), encoding: .utf8)
        #expect(!fixture.isEmpty)
        #expect(generator.contains("../../crates/client/src/testdata/{WORKFLOW_FIXTURE}"))
        #expect(generator.contains("FONTS + [ASSET_CATALOG, WORKFLOW_FIXTURE]"))
        #expect(harness.contains("Bundle.main.url(forResource: \"workflow-change-question\", withExtension: \"txt\")"))
    }
}
