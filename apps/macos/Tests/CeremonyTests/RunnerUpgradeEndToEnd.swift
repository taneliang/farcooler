import Foundation
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_UPGRADE_E2E, set by `reports/ov-984/end-to-end.py`): the
/// recovery screen's model runs the real CLI against a real scratch runner on
/// schema 60, serving recovery, from Prepare to Finish. Only Touch ID stands
/// in: a test can't put a finger on the sensor. The harness runs this test as
/// a person outside every pane, which the app is to the runner's seat check.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_UPGRADE_E2E"] != nil))
struct RunnerUpgradeEndToEnd {
    @Test func theButtonFinishesARealUpgrade() async throws {
        let home = try #require(ProcessInfo.processInfo.environment["FARCOOLER_HOME"])
        #expect(home.hasPrefix("/tmp/") || home.hasPrefix("/private/tmp/"), "a scratch runner only, never this Mac's")
        var live = RunnerUpgrade.Driver.live
        let prompts = Counter()
        live.confirm = { _ in prompts.bump(); return .yes }
        let upgrade = RunnerUpgrade(driver: live)
        await upgrade.finish()
        #expect(prompts.count == 1)
        #expect(upgrade.phase == .done, "\(upgrade.phase)")
        #expect(upgrade.steps == RunnerUpgrade.Step.allCases)
    }
}
